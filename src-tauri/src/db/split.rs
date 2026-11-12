//! Splits SQL or mongosh text into statements (for `tb query` with files and scripts).
//! A port of the editor's `src/editor/splitter.ts`, with the same rules and test cases:
//!
//! '…' "…" `…` [ … ] quoting (with '' and \' escapes), -- and # line comments, /* … */ block
//! comments, MySQL `DELIMITER` lines, and compound bodies (CREATE PROCEDURE / FUNCTION /
//! TRIGGER / EVENT … BEGIN … END) whose inner `;` must not end the statement. MongoDB
//! statements end at ";" or a line break outside brackets, unless the next line continues the
//! chain with ".".

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Mysql,
    Sqlite,
    Mongo,
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

fn starts_with(s: &[char], i: usize, pat: &[char]) -> bool {
    i + pat.len() <= s.len() && s[i..i + pat.len()] == *pat
}

fn find(s: &[char], from: usize, pat: &[char]) -> Option<usize> {
    (from..s.len()).find(|&i| starts_with(s, i, pat))
}

/// `^\s*(CREATE … PROCEDURE|FUNCTION|TRIGGER|EVENT | ALTER EVENT)`, on the statement's start.
fn is_compound(head: &str) -> bool {
    let words: Vec<String> = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_ascii_uppercase())
        .collect();
    match words.first().map(String::as_str) {
        Some("CREATE") => words[1..].iter().any(|w| matches!(w.as_str(), "PROCEDURE" | "FUNCTION" | "TRIGGER" | "EVENT")),
        Some("ALTER") => words.get(1).map(String::as_str) == Some("EVENT"),
        _ => false,
    }
}

/// `DELIMITER x` at the start of a line: the new delimiter and the length of that line.
fn delimiter_line(s: &[char], i: usize) -> Option<(String, usize)> {
    let mut j = i;
    while j < s.len() && (s[j] == ' ' || s[j] == '\t') {
        j += 1;
    }
    let word: String = s.get(j..j + 9)?.iter().collect();
    if !word.eq_ignore_ascii_case("DELIMITER") {
        return None;
    }
    j += 9;
    let ws = j;
    while j < s.len() && (s[j] == ' ' || s[j] == '\t') {
        j += 1;
    }
    if j == ws {
        return None;
    }
    let d0 = j;
    while j < s.len() && !s[j].is_whitespace() {
        j += 1;
    }
    if j == d0 {
        return None;
    }
    let delim: String = s[d0..j].iter().collect();
    while j < s.len() && (s[j] == ' ' || s[j] == '\t') {
        j += 1;
    }
    if j < s.len() && s[j] == '\r' {
        j += 1;
    }
    if j < s.len() {
        if s[j] != '\n' {
            return None;
        }
        j += 1;
    }
    Some((delim, j - i))
}

pub fn split(src: &str, dialect: Dialect) -> Vec<String> {
    if dialect == Dialect::Mongo {
        return split_mongo(src);
    }
    let s: Vec<char> = src.chars().collect();
    let n = s.len();
    let mut out = Vec::new();
    let mut delimiter: Vec<char> = vec![';'];
    let mut depth: i32 = 0;
    let mut compound: Option<bool> = None;
    let mut first_code: Option<usize> = None;
    let mut i = 0;
    let mut at_line_start = true;

    let mut push = |to: usize, first: &mut Option<usize>, depth: &mut i32, compound: &mut Option<bool>| {
        if let Some(f) = *first {
            let mut t = to;
            while t > f && s[t - 1].is_whitespace() {
                t -= 1;
            }
            out.push(s[f..t].iter().collect::<String>());
        }
        *depth = 0;
        *compound = None;
        *first = None;
    };

    while i < n {
        let c = s[i];
        if dialect == Dialect::Mysql && at_line_start && first_code.is_none() {
            if let Some((d, len)) = delimiter_line(&s, i) {
                delimiter = d.chars().collect();
                i += len;
                at_line_start = true;
                continue;
            }
        }
        if c == '\n' {
            at_line_start = true;
            i += 1;
            continue;
        }
        if c == ' ' || c == '\t' || c == '\r' {
            i += 1;
            continue;
        }
        at_line_start = false;

        // Comments.
        if c == '-' && s.get(i + 1) == Some(&'-') && (dialect == Dialect::Sqlite || i + 2 >= n || s[i + 2].is_whitespace()) {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '#' && dialect == Dialect::Mysql {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && s.get(i + 1) == Some(&'*') {
            i = find(&s, i + 2, &['*', '/']).map(|p| p + 2).unwrap_or(n);
            continue;
        }

        // Delimiter.
        if starts_with(&s, i, &delimiter) && (delimiter != [';'] || depth <= 0) {
            push(i, &mut first_code, &mut depth, &mut compound);
            i += delimiter.len();
            continue;
        }

        if first_code.is_none() {
            first_code = Some(i);
        }

        // Quoted strings and identifiers.
        if c == '\'' || c == '"' || c == '`' {
            i += 1;
            while i < n {
                if s[i] == '\\' && c != '`' && dialect == Dialect::Mysql {
                    i += 2;
                    continue;
                }
                if s[i] == c {
                    if s.get(i + 1) == Some(&c) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == '[' && dialect == Dialect::Sqlite {
            i = find(&s, i + 1, &[']']).map(|p| p + 1).unwrap_or(n);
            continue;
        }

        // Words: BEGIN/CASE … END nesting inside compound statements.
        if is_word(c) {
            let mut j = i;
            while j < n && is_word(s[j]) && !(delimiter != [';'] && starts_with(&s, j, &delimiter)) {
                j += 1;
            }
            let word: String = s[i..j].iter().collect::<String>().to_ascii_uppercase();
            if compound.is_none()
                && matches!(
                    word.as_str(),
                    "CREATE" | "ALTER" | "BEGIN" | "DECLARE" | "SELECT" | "INSERT" | "UPDATE" | "DELETE" | "WITH" | "SET" | "SHOW" | "USE" | "DROP" | "CALL"
                )
            {
                let f = first_code.unwrap_or(i);
                let head: String = s[f..(f + 400).min(n)].iter().collect();
                compound = Some(is_compound(&head));
            }
            if compound == Some(true) {
                if word == "BEGIN" || word == "CASE" {
                    depth += 1;
                } else if word == "END" {
                    // END IF / END LOOP / END WHILE / END REPEAT close blocks we don't count.
                    let rest: String = s[j..(j + 20).min(n)].iter().collect();
                    let next = rest.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next().unwrap_or("").to_ascii_uppercase();
                    let starts_ws = rest.starts_with(char::is_whitespace);
                    if !(starts_ws && matches!(next.as_str(), "IF" | "LOOP" | "WHILE" | "REPEAT")) {
                        depth -= 1;
                    }
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
    push(n, &mut first_code, &mut depth, &mut compound);
    out
}

fn split_mongo(src: &str) -> Vec<String> {
    let s: Vec<char> = src.chars().collect();
    let n = s.len();
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut first: Option<usize> = None;
    let mut i = 0;
    let mut push = |to: usize, first: &mut Option<usize>, depth: &mut i32| {
        if let Some(f) = *first {
            let mut t = to;
            while t > f && s[t - 1].is_whitespace() {
                t -= 1;
            }
            out.push(s[f..t].iter().collect::<String>());
        }
        *first = None;
        *depth = 0;
    };
    while i < n {
        let c = s[i];
        if c == '/' && s.get(i + 1) == Some(&'/') {
            while i < n && s[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && s.get(i + 1) == Some(&'*') {
            i = find(&s, i + 2, &['*', '/']).map(|p| p + 2).unwrap_or(n);
            continue;
        }
        if c == '\n' && depth <= 0 && first.is_some() {
            let mut j = i + 1;
            while j < n && s[j].is_whitespace() {
                j += 1;
            }
            if s.get(j) != Some(&'.') {
                push(i, &mut first, &mut depth);
                i += 1;
                continue;
            }
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == ';' && depth <= 0 {
            push(i, &mut first, &mut depth);
            i += 1;
            continue;
        }
        if first.is_none() {
            first = Some(i);
        }
        if c == '\'' || c == '"' || c == '`' {
            i += 1;
            while i < n && s[i] != c {
                i += if s[i] == '\\' { 2 } else { 1 };
            }
            i += 1;
            continue;
        }
        if c == '(' || c == '[' || c == '{' {
            depth += 1;
        } else if c == ')' || c == ']' || c == '}' {
            depth -= 1;
        }
        i += 1;
    }
    push(n, &mut first, &mut depth);
    out
}

#[cfg(test)]
mod tests {
    use super::{Dialect, split};

    fn t(sql: &str) -> Vec<String> {
        split(sql, Dialect::Mysql)
    }

    #[test]
    fn basic() {
        assert_eq!(t("SELECT 1; SELECT 2;\nSELECT 3"), ["SELECT 1", "SELECT 2", "SELECT 3"]);
    }

    #[test]
    fn semicolons_in_strings_identifiers_and_comments() {
        assert_eq!(
            t("SELECT 'a;b', \"c;d\", `e;f` -- x;y\n; # z;w\nSELECT /* ; */ 2;"),
            ["SELECT 'a;b', \"c;d\", `e;f` -- x;y", "SELECT /* ; */ 2"]
        );
    }

    #[test]
    fn escaped_quotes() {
        assert_eq!(t("SELECT 'it''s; ok', 'a\\';b'; SELECT 2"), ["SELECT 'it''s; ok', 'a\\';b'", "SELECT 2"]);
        assert_eq!(split("SELECT 'a\\'; SELECT 2", Dialect::Sqlite), ["SELECT 'a\\'", "SELECT 2"]);
    }

    #[test]
    fn mysql_comments() {
        assert_eq!(t("SELECT 5--1; SELECT 2"), ["SELECT 5--1", "SELECT 2"]);
        assert_eq!(t("SELECT 1 #; comment\n; SELECT 2"), ["SELECT 1 #; comment", "SELECT 2"]);
        assert_eq!(t("-- just a note\n;\n/* x */;\nSELECT 1;"), ["SELECT 1"]);
    }

    #[test]
    fn delimiter_blocks() {
        let sql = "DELIMITER $$\nCREATE PROCEDURE p()\nBEGIN\n  SELECT 1; -- inside\n  SELECT 2;\nEND$$\nDELIMITER ;\nCALL p();";
        let v = t(sql);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].starts_with("CREATE PROCEDURE p()") && v[0].ends_with("END"));
        assert_eq!(v[1], "CALL p()");
    }

    #[test]
    fn compound_body_without_delimiter() {
        let sql = "CREATE TRIGGER t AFTER INSERT ON x FOR EACH ROW BEGIN\n IF NEW.a > 1 THEN SET @y = 1; END IF;\n UPDATE z SET b = CASE WHEN 1 THEN 2 END;\nEND;\nSELECT 3;";
        let v = t(sql);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].ends_with("END"));
        assert_eq!(v[1], "SELECT 3");
    }

    #[test]
    fn begin_as_transaction() {
        assert_eq!(t("BEGIN; UPDATE a SET b = 1; COMMIT;"), ["BEGIN", "UPDATE a SET b = 1", "COMMIT"]);
    }

    #[test]
    fn sqlite_trigger_and_brackets() {
        let sql = "CREATE TRIGGER tr AFTER DELETE ON [my;table] BEGIN DELETE FROM b WHERE id = old.id; END; SELECT 1;";
        assert_eq!(split(sql, Dialect::Sqlite).len(), 2);
    }

    #[test]
    fn many_statements() {
        let sql: Vec<String> = (0..200).map(|i| format!("INSERT INTO t VALUES ({i}, 'a;b');")).collect();
        assert_eq!(t(&sql.join("\n")).len(), 200);
    }

    #[test]
    fn mongo_chains_and_objects() {
        let src = "db.users.find({ age: { $gt: 30 } })\n  .sort({ name: 1 })\n  .limit(5)\n// a comment; with a semicolon\ndb.orders.aggregate([\n  { $match: { status: 'paid; or not' } },\n  { $group: { _id: '$user', n: { $sum: 1 } } }\n])\nshow collections; db.x.countDocuments({})";
        let v = split(src, Dialect::Mongo);
        assert_eq!(v.len(), 4, "{v:?}");
        assert!(v[0].starts_with("db.users.find") && v[0].ends_with(".limit(5)"));
        assert!(v[1].starts_with("db.orders.aggregate([") && v[1].ends_with("])"));
        assert_eq!(v[2], "show collections");
        assert_eq!(v[3], "db.x.countDocuments({})");
    }

    #[test]
    fn non_ascii_text() {
        assert_eq!(t("SELECT 'héllo; wörld'; SELECT 'ünïcode'"), ["SELECT 'héllo; wörld'", "SELECT 'ünïcode'"]);
    }
}
