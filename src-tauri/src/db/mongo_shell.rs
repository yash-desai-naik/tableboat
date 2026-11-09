//! Parses mongosh-style statements into commands, without running JavaScript:
//!   db.users.find({ age: { $gt: 30 } }, { name: 1 }).sort({ name: 1 }).skip(10).limit(5)
//!   db.getCollection("orders").aggregate([{ $match: { status: 'paid' } }])
//!   db.users.updateOne({ _id: ObjectId("…") }, { $set: { active: true } })
//!   show dbs | show collections | use shop | db.runCommand({ ping: 1 })
//! Values are JavaScript literals: unquoted keys, '…' or "…" strings, numbers, true/false/null,
//! ObjectId(), ISODate()/new Date(), NumberInt/NumberLong/NumberDecimal(), UUID(), Timestamp(),
//! MinKey/MaxKey and /regex/flags. `//` and `/* */` comments are ignored.

use mongodb::bson::{self, Bson, Document, oid::ObjectId};

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    ShowDatabases,
    ShowCollections,
    Use(String),
    RunCommand { admin: bool, command: Document },
    CollectionNames,
    Collection { name: String, op: Op },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Find { filter: Document, projection: Option<Document>, sort: Option<Document>, skip: Option<u64>, limit: Option<i64>, one: bool, count: bool },
    Aggregate { pipeline: Vec<Document> },
    CountDocuments { filter: Document },
    EstimatedCount,
    Distinct { field: String, filter: Document },
    InsertOne { doc: Document },
    InsertMany { docs: Vec<Document> },
    UpdateOne { filter: Document, update: Bson, upsert: bool },
    UpdateMany { filter: Document, update: Bson, upsert: bool },
    ReplaceOne { filter: Document, doc: Document, upsert: bool },
    DeleteOne { filter: Document },
    DeleteMany { filter: Document },
    CreateIndex { keys: Document, options: Option<Document> },
    DropIndex { name: String },
    GetIndexes,
    Drop,
}

impl Op {
    /// True when the operation only reads.
    pub fn is_read_only(&self) -> bool {
        match self {
            Op::Find { .. } | Op::CountDocuments { .. } | Op::EstimatedCount | Op::Distinct { .. } | Op::GetIndexes => true,
            // $out and $merge write the pipeline's output to a collection.
            Op::Aggregate { pipeline } => !pipeline.iter().any(|s| s.contains_key("$out") || s.contains_key("$merge")),
            _ => false,
        }
    }
}

// ---------- tokenizer-free recursive descent over chars ----------

struct P<'a> {
    s: &'a [char],
    i: usize,
}

type R<T> = Result<T, String>;

impl<'a> P<'a> {
    fn err<T>(&self, msg: &str) -> R<T> {
        let ctx: String = self.s[self.i.min(self.s.len())..].iter().take(30).collect();
        Err(if ctx.is_empty() { format!("{msg} at the end") } else { format!("{msg} near \"{ctx}\"") })
    }

    fn skip_ws(&mut self) {
        loop {
            while self.i < self.s.len() && self.s[self.i].is_whitespace() {
                self.i += 1;
            }
            if self.peek_str("//") {
                while self.i < self.s.len() && self.s[self.i] != '\n' {
                    self.i += 1;
                }
                continue;
            }
            if self.peek_str("/*") {
                self.i += 2;
                while self.i < self.s.len() && !self.peek_str("*/") {
                    self.i += 1;
                }
                self.i = (self.i + 2).min(self.s.len());
                continue;
            }
            break;
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.skip_ws();
        self.s.get(self.i).copied()
    }

    fn peek_str(&self, t: &str) -> bool {
        let t: Vec<char> = t.chars().collect();
        self.s.len() >= self.i + t.len() && self.s[self.i..self.i + t.len()] == t[..]
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> R<()> {
        if self.eat(c) { Ok(()) } else { self.err(&format!("Expected '{c}'")) }
    }

    fn ident(&mut self) -> Option<String> {
        self.skip_ws();
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_alphanumeric() || self.s[self.i] == '_' || self.s[self.i] == '$') {
            self.i += 1;
        }
        (self.i > start).then(|| self.s[start..self.i].iter().collect())
    }

    fn string(&mut self) -> R<String> {
        let q = self.peek().ok_or("Expected a string")?;
        if q != '"' && q != '\'' && q != '`' {
            return self.err("Expected a string");
        }
        self.i += 1;
        let mut out = String::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            if c == q {
                return Ok(out);
            }
            if c == '\\' && self.i < self.s.len() {
                let e = self.s[self.i];
                self.i += 1;
                match e {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    '0' => out.push('\0'),
                    'u' => {
                        let hex: String = self.s[self.i..(self.i + 4).min(self.s.len())].iter().collect();
                        let code = u32::from_str_radix(&hex, 16).map_err(|_| "Bad \\u escape".to_string())?;
                        out.push(char::from_u32(code).unwrap_or('\u{fffd}'));
                        self.i += 4;
                    }
                    other => out.push(other),
                }
            } else {
                out.push(c);
            }
        }
        self.err("Unterminated string")
    }

    fn number(&mut self) -> R<Bson> {
        self.skip_ws();
        let start = self.i;
        if matches!(self.s.get(self.i), Some('-') | Some('+')) {
            self.i += 1;
        }
        while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], '.' | 'e' | 'E' | '_')) {
            if matches!(self.s[self.i], 'e' | 'E') && matches!(self.s.get(self.i + 1), Some('-') | Some('+')) {
                self.i += 1;
            }
            self.i += 1;
        }
        let text: String = self.s[start..self.i].iter().filter(|c| **c != '_').collect();
        if text.is_empty() || text == "-" || text == "+" {
            return self.err("Expected a value");
        }
        if !text.contains(['.', 'e', 'E']) {
            if let Ok(n) = text.parse::<i64>() {
                return Ok(i32::try_from(n).map(Bson::Int32).unwrap_or(Bson::Int64(n)));
            }
        }
        text.parse::<f64>().map(Bson::Double).or_else(|_| self.err("Bad number"))
    }

    fn args(&mut self) -> R<Vec<Bson>> {
        self.expect('(')?;
        let mut out = Vec::new();
        if self.eat(')') {
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            if self.eat(')') {
                return Ok(out);
            }
            self.expect(',')?;
            if self.eat(')') {
                return Ok(out);
            }
        }
    }

    fn value(&mut self) -> R<Bson> {
        match self.peek() {
            None => self.err("Expected a value"),
            Some('{') => Ok(Bson::Document(self.document()?)),
            Some('[') => {
                self.i += 1;
                let mut arr = Vec::new();
                loop {
                    if self.eat(']') {
                        return Ok(Bson::Array(arr));
                    }
                    arr.push(self.value()?);
                    if !self.eat(',') {
                        self.expect(']')?;
                        return Ok(Bson::Array(arr));
                    }
                }
            }
            Some('"') | Some('\'') | Some('`') => Ok(Bson::String(self.string()?)),
            Some('/') => self.regex(),
            Some(c) if c.is_ascii_digit() || c == '-' || c == '+' || c == '.' => self.number(),
            Some(_) => {
                let name = self.ident().ok_or_else(|| self.err::<()>("Unexpected character").unwrap_err())?;
                match name.as_str() {
                    "true" => Ok(Bson::Boolean(true)),
                    "false" => Ok(Bson::Boolean(false)),
                    "null" => Ok(Bson::Null),
                    "undefined" => Ok(Bson::Undefined),
                    "MinKey" => self.optional_call(Bson::MinKey),
                    "MaxKey" => self.optional_call(Bson::MaxKey),
                    "Infinity" => Ok(Bson::Double(f64::INFINITY)),
                    "NaN" => Ok(Bson::Double(f64::NAN)),
                    "new" => {
                        let ctor = self.ident().ok_or("Expected a name after 'new'")?;
                        self.call(&ctor)
                    }
                    other => self.call(other),
                }
            }
        }
    }

    fn optional_call(&mut self, v: Bson) -> R<Bson> {
        if self.peek() == Some('(') {
            self.args()?;
        }
        Ok(v)
    }

    fn call(&mut self, name: &str) -> R<Bson> {
        let args = self.args()?;
        let str_arg = |i: usize| -> R<String> {
            match args.get(i) {
                Some(Bson::String(s)) => Ok(s.clone()),
                Some(Bson::Int32(n)) => Ok(n.to_string()),
                Some(Bson::Int64(n)) => Ok(n.to_string()),
                Some(Bson::Double(n)) => Ok(n.to_string()),
                _ => Err(format!("{name}() needs a value")),
            }
        };
        match name {
            "ObjectId" => {
                if args.is_empty() {
                    return Ok(Bson::ObjectId(ObjectId::new()));
                }
                let s = str_arg(0)?;
                ObjectId::parse_str(&s).map(Bson::ObjectId).map_err(|_| format!("\"{s}\" is not a valid ObjectId"))
            }
            "ISODate" | "Date" => {
                if args.is_empty() {
                    return Ok(Bson::DateTime(bson::DateTime::now()));
                }
                match &args[0] {
                    Bson::Int32(n) => Ok(Bson::DateTime(bson::DateTime::from_millis(*n as i64))),
                    Bson::Int64(n) => Ok(Bson::DateTime(bson::DateTime::from_millis(*n))),
                    Bson::Double(n) => Ok(Bson::DateTime(bson::DateTime::from_millis(*n as i64))),
                    _ => parse_date(&str_arg(0)?).map(Bson::DateTime),
                }
            }
            "NumberInt" | "Int32" => str_arg(0)?.trim().parse::<i32>().map(Bson::Int32).map_err(|_| "NumberInt() needs a whole number".into()),
            "NumberLong" | "Long" => str_arg(0)?.trim().parse::<i64>().map(Bson::Int64).map_err(|_| "NumberLong() needs a whole number".into()),
            "NumberDecimal" | "Decimal128" => decimal(&str_arg(0)?),
            "UUID" => uuid(&str_arg(0)?),
            "Timestamp" => {
                let t = args.first().and_then(as_u32).unwrap_or(0);
                let i = args.get(1).and_then(as_u32).unwrap_or(0);
                Ok(Bson::Timestamp(bson::Timestamp { time: t, increment: i }))
            }
            "RegExp" => Ok(regex(str_arg(0)?, args.get(1).and_then(Bson::as_str).unwrap_or(""))),
            other => Err(format!("{other}() isn't supported in Tableboat's MongoDB syntax")),
        }
    }

    fn regex(&mut self) -> R<Bson> {
        self.i += 1; // '/'
        let mut pattern = String::new();
        let mut in_class = false;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                '\\' => {
                    pattern.push(c);
                    if let Some(n) = self.s.get(self.i) {
                        pattern.push(*n);
                        self.i += 1;
                    }
                }
                '[' => {
                    in_class = true;
                    pattern.push(c);
                }
                ']' => {
                    in_class = false;
                    pattern.push(c);
                }
                '/' if !in_class => {
                    let mut flags = String::new();
                    while self.i < self.s.len() && self.s[self.i].is_ascii_alphabetic() {
                        flags.push(self.s[self.i]);
                        self.i += 1;
                    }
                    return Ok(regex(pattern, &flags));
                }
                _ => pattern.push(c),
            }
        }
        self.err("Unterminated regular expression")
    }

    fn document(&mut self) -> R<Document> {
        self.expect('{')?;
        let mut doc = Document::new();
        loop {
            if self.eat('}') {
                return Ok(doc);
            }
            let key = match self.peek() {
                Some('"') | Some('\'') | Some('`') => self.string()?,
                _ => {
                    // Unquoted keys may contain dots (a.b) as many people write them.
                    self.skip_ws();
                    let start = self.i;
                    while self.i < self.s.len()
                        && (self.s[self.i].is_alphanumeric() || matches!(self.s[self.i], '_' | '$' | '.'))
                    {
                        self.i += 1;
                    }
                    if self.i == start {
                        return self.err("Expected a field name");
                    }
                    self.s[start..self.i].iter().collect()
                }
            };
            self.expect(':')?;
            let value = self.value()?;
            doc.insert(key, value);
            if !self.eat(',') {
                self.expect('}')?;
                return Ok(doc);
            }
        }
    }
}

/// A BSON regex (MongoDB expects the option letters sorted).
fn regex(pattern: String, flags: &str) -> Bson {
    let mut options: Vec<char> = flags.chars().collect();
    options.sort_unstable();
    options.dedup();
    Bson::RegularExpression(bson::Regex { pattern, options: options.into_iter().collect() })
}

fn as_u32(b: &Bson) -> Option<u32> {
    match b {
        Bson::Int32(n) => u32::try_from(*n).ok(),
        Bson::Int64(n) => u32::try_from(*n).ok(),
        Bson::Double(n) => Some(*n as u32),
        _ => None,
    }
}

pub fn parse_date(s: &str) -> R<bson::DateTime> {
    let s = s.trim();
    if let Ok(d) = bson::DateTime::parse_rfc3339_str(s) {
        return Ok(d);
    }
    // "2024-01-31" or "2024-01-31 10:20:30" (taken as UTC, like mongosh).
    let fixed = if s.len() == 10 {
        format!("{s}T00:00:00Z")
    } else {
        let t = s.replacen(' ', "T", 1);
        if t.ends_with('Z') || t[10..].contains(['+', '-']) { t } else { format!("{t}Z") }
    };
    bson::DateTime::parse_rfc3339_str(&fixed).map_err(|_| format!("\"{s}\" is not a date (use e.g. 2024-01-31T10:20:30Z)"))
}

fn decimal(s: &str) -> R<Bson> {
    let ext = serde_json::json!({ "$numberDecimal": s.trim() });
    Bson::try_from(ext).map_err(|_| format!("\"{s}\" is not a valid decimal"))
}

fn uuid(s: &str) -> R<Bson> {
    let u = uuid::Uuid::parse_str(s.trim()).map_err(|_| format!("\"{s}\" is not a valid UUID"))?;
    Ok(Bson::Binary(bson::Binary { subtype: bson::spec::BinarySubtype::Uuid, bytes: u.as_bytes().to_vec() }))
}

/// Parse one JavaScript-literal value (used for edited cells and inserted values).
pub fn parse_value(text: &str) -> R<Bson> {
    let chars: Vec<char> = text.chars().collect();
    let mut p = P { s: &chars, i: 0 };
    let v = p.value()?;
    if p.peek().is_some() {
        return p.err("Unexpected text after the value");
    }
    Ok(v)
}

fn doc_arg(args: &[Bson], i: usize, what: &str) -> R<Document> {
    match args.get(i) {
        None => Ok(Document::new()),
        Some(Bson::Document(d)) => Ok(d.clone()),
        Some(_) => Err(format!("The {what} must be an object {{ … }}")),
    }
}

fn required_doc(args: &[Bson], i: usize, what: &str) -> R<Document> {
    match args.get(i) {
        Some(Bson::Document(d)) => Ok(d.clone()),
        _ => Err(format!("Give the {what} as an object {{ … }}")),
    }
}

fn upsert(args: &[Bson], i: usize) -> bool {
    matches!(args.get(i), Some(Bson::Document(d)) if d.get_bool("upsert").unwrap_or(false))
}

/// Parse one statement.
pub fn parse(statement: &str) -> R<Command> {
    let text = statement.trim().trim_end_matches(';').trim();
    let lower = text.to_ascii_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    match words.as_slice() {
        ["show", "dbs"] | ["show", "databases"] => return Ok(Command::ShowDatabases),
        ["show", "collections"] | ["show", "tables"] => return Ok(Command::ShowCollections),
        ["use", _] => return Ok(Command::Use(text.split_whitespace().nth(1).unwrap().to_string())),
        _ => {}
    }

    let chars: Vec<char> = text.chars().collect();
    let mut p = P { s: &chars, i: 0 };
    if p.ident().as_deref() != Some("db") {
        return Err("Start with db.<collection>.<method>(…), e.g. db.users.find({ age: { $gt: 30 } })".into());
    }
    p.expect('.')?;
    let first = p.ident().ok_or("Expected a collection name or method after db.")?;

    // db.method(…)
    if p.peek() == Some('(') {
        let args = p.args()?;
        let cmd = match first.as_str() {
            "runCommand" => Command::RunCommand { admin: false, command: required_doc(&args, 0, "command")? },
            "adminCommand" => Command::RunCommand { admin: true, command: required_doc(&args, 0, "command")? },
            "getCollectionNames" => Command::CollectionNames,
            "stats" => Command::RunCommand { admin: false, command: bson::doc! { "dbStats": 1 } },
            "getCollection" => {
                let name = match args.first() {
                    Some(Bson::String(s)) => s.clone(),
                    _ => return Err("db.getCollection() needs a collection name".into()),
                };
                p.expect('.')?;
                let method = p.ident().ok_or("Expected a method")?;
                return collection_op(&mut p, name, &method);
            }
            other => return Err(format!("db.{other}() isn't supported")),
        };
        if p.peek().is_some() {
            return p.err("Unexpected text");
        }
        return Ok(cmd);
    }

    // db.collection.method(…) — collection names may contain dots (db.app.logs.find()).
    let mut name = first;
    loop {
        p.expect('.')?;
        let part = p.ident().ok_or("Expected a method name")?;
        if p.peek() == Some('(') {
            return collection_op(&mut p, name, &part);
        }
        name = format!("{name}.{part}");
    }
}

fn collection_op(p: &mut P, name: String, method: &str) -> R<Command> {
    let args = p.args()?;
    let mut op = match method {
        "find" | "findOne" => Op::Find {
            filter: doc_arg(&args, 0, "filter")?,
            projection: args.get(1).map(|_| doc_arg(&args, 1, "projection")).transpose()?,
            sort: None,
            skip: None,
            limit: if method == "findOne" { Some(1) } else { None },
            one: method == "findOne",
            count: false,
        },
        "aggregate" => {
            let pipeline = match args.first() {
                Some(Bson::Array(stages)) => stages
                    .iter()
                    .map(|s| s.as_document().cloned().ok_or_else(|| "Each pipeline stage must be an object".to_string()))
                    .collect::<R<Vec<_>>>()?,
                None => vec![],
                _ => return Err("aggregate() needs an array of stages: aggregate([ { $match: … } ])".into()),
            };
            Op::Aggregate { pipeline }
        }
        "countDocuments" | "count" => Op::CountDocuments { filter: doc_arg(&args, 0, "filter")? },
        "estimatedDocumentCount" => Op::EstimatedCount,
        "distinct" => Op::Distinct {
            field: args.first().and_then(Bson::as_str).ok_or("distinct() needs a field name")?.to_string(),
            filter: doc_arg(&args, 1, "filter")?,
        },
        "insertOne" | "insert" if matches!(args.first(), Some(Bson::Document(_))) => {
            Op::InsertOne { doc: required_doc(&args, 0, "document")? }
        }
        "insertMany" | "insert" => match args.first() {
            Some(Bson::Array(items)) => Op::InsertMany {
                docs: items
                    .iter()
                    .map(|d| d.as_document().cloned().ok_or_else(|| "insertMany() needs an array of documents".to_string()))
                    .collect::<R<Vec<_>>>()?,
            },
            _ => return Err("insertMany() needs an array of documents".into()),
        },
        "updateOne" | "updateMany" => {
            let update = match args.get(1) {
                Some(b @ Bson::Document(_)) | Some(b @ Bson::Array(_)) => b.clone(),
                _ => return Err(format!("{method}() needs a filter and an update, e.g. {{ $set: {{ … }} }}")),
            };
            let filter = required_doc(&args, 0, "filter")?;
            let upsert = upsert(&args, 2);
            if method == "updateOne" { Op::UpdateOne { filter, update, upsert } } else { Op::UpdateMany { filter, update, upsert } }
        }
        "replaceOne" => Op::ReplaceOne {
            filter: required_doc(&args, 0, "filter")?,
            doc: required_doc(&args, 1, "replacement document")?,
            upsert: upsert(&args, 2),
        },
        "deleteOne" | "remove" if method == "deleteOne" => Op::DeleteOne { filter: required_doc(&args, 0, "filter")? },
        "deleteMany" | "remove" => Op::DeleteMany { filter: required_doc(&args, 0, "filter")? },
        "createIndex" => Op::CreateIndex {
            keys: required_doc(&args, 0, "index keys")?,
            options: args.get(1).map(|_| doc_arg(&args, 1, "options")).transpose()?,
        },
        "dropIndex" => Op::DropIndex { name: args.first().and_then(Bson::as_str).ok_or("dropIndex() needs the index name")?.to_string() },
        "getIndexes" => Op::GetIndexes,
        "drop" => Op::Drop,
        other => return Err(format!("{other}() isn't supported on collections")),
    };

    // Chained cursor methods.
    while p.eat('.') {
        let m = p.ident().ok_or("Expected a method after '.'")?;
        let a = p.args()?;
        match (&mut op, m.as_str()) {
            (Op::Find { sort, .. }, "sort") => *sort = Some(required_doc(&a, 0, "sort")?),
            (Op::Find { skip, .. }, "skip") => *skip = a.first().and_then(as_u32).map(u64::from),
            (Op::Find { limit, .. }, "limit") => *limit = a.first().and_then(|b| b.as_i64().or(b.as_i32().map(i64::from))),
            (Op::Find { projection, .. }, "project" | "projection") => *projection = Some(required_doc(&a, 0, "projection")?),
            (Op::Find { count, .. }, "count" | "itcount" | "size") => *count = true,
            (_, "pretty" | "toArray" | "explain" | "batchSize" | "maxTimeMS" | "hint" | "comment") => {}
            (_, other) => return Err(format!(".{other}() isn't supported here")),
        }
    }
    if p.peek().is_some() {
        return p.err("Unexpected text");
    }
    Ok(Command::Collection { name, op })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn find_with_chain() {
        let c = parse("db.users.find({ age: { $gt: 30 }, 'name': /^jo/i }, { name: 1 }).sort({ name: -1 }).skip(5).limit(10);").unwrap();
        let Command::Collection { name, op: Op::Find { filter, projection, sort, skip, limit, .. } } = c else { panic!("{c:?}") };
        assert_eq!(name, "users");
        assert_eq!(filter.get_document("age").unwrap(), &doc! { "$gt": 30 });
        assert!(matches!(filter.get("name"), Some(Bson::RegularExpression(r)) if r.pattern == "^jo" && r.options == "i"));
        assert_eq!(projection, Some(doc! { "name": 1 }));
        assert_eq!(sort, Some(doc! { "name": -1 }));
        assert_eq!((skip, limit), (Some(5), Some(10)));
    }

    #[test]
    fn values() {
        assert!(matches!(parse_value(r#"ObjectId("507f1f77bcf86cd799439011")"#).unwrap(), Bson::ObjectId(_)));
        assert!(matches!(parse_value("ISODate('2024-01-31T10:00:00Z')").unwrap(), Bson::DateTime(_)));
        assert!(matches!(parse_value("new Date('2024-01-31')").unwrap(), Bson::DateTime(_)));
        assert_eq!(parse_value("NumberLong(5)").unwrap(), Bson::Int64(5));
        assert_eq!(parse_value("3000000000").unwrap(), Bson::Int64(3_000_000_000));
        assert_eq!(parse_value("1.5").unwrap(), Bson::Double(1.5));
        assert!(matches!(parse_value("NumberDecimal('12.50')").unwrap(), Bson::Decimal128(_)));
        assert!(matches!(parse_value("[1, 'a', {x: null},]").unwrap(), Bson::Array(a) if a.len() == 3));
        assert!(parse_value("{ a: 1 } junk").is_err());
    }

    #[test]
    fn other_forms() {
        assert_eq!(parse("show dbs").unwrap(), Command::ShowDatabases);
        assert_eq!(parse("use shop").unwrap(), Command::Use("shop".into()));
        assert!(matches!(parse("db.getCollection('my.logs').countDocuments({})").unwrap(), Command::Collection { name, .. } if name == "my.logs"));
        assert!(matches!(parse("db.app.logs.find()").unwrap(), Command::Collection { name, .. } if name == "app.logs"));
        assert!(matches!(parse("db.runCommand({ ping: 1 })").unwrap(), Command::RunCommand { admin: false, .. }));
        let Command::Collection { op, .. } = parse("db.o.aggregate([{ $match: { s: 'paid' } }, { $out: 'x' }])").unwrap() else { panic!() };
        assert!(!op.is_read_only(), "$out writes");
        let Command::Collection { op, .. } = parse("db.o.updateMany({}, { $set: { a: 1 } }, { upsert: true })").unwrap() else { panic!() };
        assert!(matches!(op, Op::UpdateMany { upsert: true, .. }) && !op.is_read_only());
        assert!(parse("users.find()").is_err());
        assert!(parse("db.users.find({ a: 1 }").is_err());
        // comments
        assert!(parse("// latest\ndb.users.find({}) /* all */ .limit(1)").is_ok());
    }
}
