//! Query results held in Rust. The grid fetches only the rows it draws; sorting, quick
//! search, copy and export all work on the rows here, so big results never sit in the
//! WebView.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use mongodb::bson::{Bson, Document, RawDocumentBuf};

/// One value as text (`None` = SQL NULL). Binary values are hex (`0x…`).
pub type Cell = Option<Box<str>>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub name: String,
    /// e.g. "int unsigned", "varchar", "datetime", "json".
    pub type_name: String,
    /// The table the column comes from, when the server says.
    pub table: Option<String>,
    pub numeric: bool,
    pub binary: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct ResultSet {
    pub columns: Vec<Column>,
    rows: Vec<Box<[Cell]>>,
    /// MongoDB: the documents themselves (row `i` is document `i` shown as columns). The
    /// document view, JSON copies and exports use these, so nesting and types are kept.
    docs: Option<Vec<RawDocumentBuf>>,
    pub truncated: bool,
    /// Row order after quick search and sort: indices into `rows`.
    view: Vec<u32>,
    sort: Option<(usize, bool)>,
    filter: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultInfo {
    pub id: String,
    pub columns: Vec<Column>,
    pub row_count: usize,
    pub truncated: bool,
    /// The rows are MongoDB documents (the document view is available).
    pub documents: bool,
}

impl ResultSet {
    pub fn new(columns: Vec<Column>, rows: Vec<Box<[Cell]>>, truncated: bool) -> Self {
        let view = (0..rows.len() as u32).collect();
        Self { columns, rows, docs: None, truncated, view, sort: None, filter: None }
    }

    /// Keep the documents the rows were made from (one per row, same order).
    pub fn with_documents(mut self, docs: Vec<RawDocumentBuf>) -> Self {
        if docs.len() == self.rows.len() {
            self.docs = Some(docs);
        }
        self
    }

    pub fn has_documents(&self) -> bool {
        self.docs.is_some()
    }

    /// The document at a position in the current view (after quick search and sort).
    pub fn document(&self, view_index: usize) -> Option<Document> {
        let i = *self.view.get(view_index)? as usize;
        Document::try_from(self.docs.as_ref()?.get(i)?.as_ref()).ok()
    }

    /// Documents `start..start+count` of the current view, as canonical Extended JSON (every
    /// type spelled out, numbers as text so nothing is rounded in JavaScript).
    pub fn documents(&self, start: usize, count: usize) -> Vec<serde_json::Value> {
        (start..start + count)
            .map_while(|i| self.document(i))
            .map(|d| Bson::Document(d).into_canonical_extjson())
            .collect()
    }

    /// Column names as JSON keys: a repeated name (a join's two `id`s) gets " (2)", " (3)"…
    fn json_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::with_capacity(self.columns.len());
        for c in &self.columns {
            let mut k = c.name.clone();
            let mut n = 2;
            while keys.contains(&k) {
                k = format!("{} ({n})", c.name);
                n += 1;
            }
            keys.push(k);
        }
        keys
    }

    /// The JSON view: rows (or documents) `start..start+count` of the current view as objects.
    /// MongoDB: the documents (canonical Extended JSON). SQL: column → value, NULL as null,
    /// JSON columns as nested values, numbers as `{"$number": "…"}` so JavaScript never
    /// rounds them.
    pub fn objects(&self, start: usize, count: usize) -> Vec<serde_json::Value> {
        if self.docs.is_some() {
            return self.documents(start, count);
        }
        let keys = self.json_keys();
        self.view
            .iter()
            .skip(start)
            .take(count)
            .map(|&i| {
                let row = &self.rows[i as usize];
                let mut obj = serde_json::Map::new();
                for (c, col) in self.columns.iter().enumerate() {
                    let v = match cell(row, c) {
                        None => serde_json::Value::Null,
                        Some(s) if col.numeric && is_plain_number(s) => serde_json::json!({ "$number": s }),
                        Some(s) if col.type_name == "json" => {
                            serde_json::from_str(s).unwrap_or_else(|_| serde_json::Value::String(s.to_string()))
                        }
                        Some(s) => serde_json::Value::String(s.to_string()),
                    };
                    obj.insert(keys[c].clone(), v);
                }
                serde_json::Value::Object(obj)
            })
            .collect()
    }

    /// One row as an indented JSON object (numbers exactly as the server sent them).
    pub fn row_json(&self, view_index: usize) -> Option<String> {
        let row = &self.rows[*self.view.get(view_index)? as usize];
        let keys = self.json_keys();
        let fields: Vec<String> = self
            .columns
            .iter()
            .enumerate()
            .map(|(c, col)| format!("  {}: {}", serde_json::to_string(&keys[c]).unwrap_or_default(), json_value(col, cell(row, c))))
            .collect();
        Some(format!("{{
{}
}}", fields.join(",
")))
    }

    pub fn visible_len(&self) -> usize {
        self.view.len()
    }

    fn recompute_view(&mut self) {
        let mut view: Vec<u32> = match &self.filter {
            Some(f) if !f.is_empty() => {
                let needle = f.to_lowercase();
                (0..self.rows.len() as u32)
                    .filter(|&i| {
                        self.rows[i as usize]
                            .iter()
                            .any(|c| c.as_deref().map(|v| v.to_lowercase().contains(&needle)).unwrap_or(false))
                    })
                    .collect()
            }
            _ => (0..self.rows.len() as u32).collect(),
        };
        if let Some((col, desc)) = self.sort {
            let numeric = self.columns.get(col).map(|c| c.numeric).unwrap_or(false);
            let rows = &self.rows;
            view.sort_by(|&a, &b| {
                let x = rows[a as usize].get(col).and_then(|c| c.as_deref());
                let y = rows[b as usize].get(col).and_then(|c| c.as_deref());
                let ord = compare_cells(x, y, numeric);
                if desc { ord.reverse() } else { ord }
            });
        }
        self.view = view;
    }

    pub fn set_sort(&mut self, sort: Option<(usize, bool)>) {
        self.sort = sort.filter(|(c, _)| *c < self.columns.len());
        self.recompute_view();
    }

    pub fn set_filter(&mut self, filter: Option<String>) -> usize {
        self.filter = filter.filter(|f| !f.trim().is_empty());
        self.recompute_view();
        self.view.len()
    }

    /// Rows `start..start+count` of the current view.
    pub fn rows(&self, start: usize, count: usize) -> Vec<Vec<Option<&str>>> {
        self.view
            .iter()
            .skip(start)
            .take(count)
            .map(|&i| self.rows[i as usize].iter().map(|c| c.as_deref()).collect())
            .collect()
    }

    /// Original row indices of view positions in `ranges`.
    fn view_indices(&self, ranges: &[[u32; 2]]) -> Vec<u32> {
        let mut out = Vec::new();
        for [start, end] in ranges {
            for v in *start..(*end).min(self.view.len() as u32) {
                out.push(self.view[v as usize]);
            }
        }
        out
    }

    pub fn all_view_rows(&self) -> Vec<&[Cell]> {
        self.view.iter().map(|&i| &*self.rows[i as usize]).collect()
    }
}

/// NULL first, numbers by value (exact, any length), text case-insensitively.
fn compare_cells(a: Option<&str>, b: Option<&str>, numeric: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => {
            if numeric {
                if let Some(o) = compare_decimal(x, y) {
                    return o;
                }
            }
            x.to_lowercase().cmp(&y.to_lowercase()).then_with(|| x.cmp(y))
        }
    }
}

/// Compare decimal strings like "-12.50" and "3" without converting to floating point,
/// so 20-digit integers and DECIMAL(38,10) values sort correctly.
fn compare_decimal(a: &str, b: &str) -> Option<Ordering> {
    fn parts(s: &str) -> Option<(bool, &str, &str)> {
        let s = s.trim();
        let (neg, s) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s.strip_prefix('+').unwrap_or(s)),
        };
        let (int, frac) = s.split_once('.').unwrap_or((s, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().all(|c| c.is_ascii_digit()) || !frac.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let int = int.trim_start_matches('0');
        let frac = frac.trim_end_matches('0');
        let zero = int.is_empty() && frac.is_empty();
        Some((neg && !zero, int, frac))
    }
    let (na, ia, fa) = parts(a)?;
    let (nb, ib, fb) = parts(b)?;
    let magnitude = ia.len().cmp(&ib.len()).then_with(|| ia.cmp(ib)).then_with(|| fa.cmp(fb));
    Some(match (na, nb) {
        (false, false) => magnitude,
        (true, true) => magnitude.reverse(),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
    })
}

// ---------- store ----------

type Shared = Arc<RwLock<ResultSet>>;

#[derive(Default)]
pub struct ResultStore {
    map: Mutex<HashMap<String, (String, Shared)>>,
}

impl ResultStore {
    pub fn insert(&self, window: &str, rs: ResultSet) -> ResultInfo {
        let id = uuid::Uuid::new_v4().to_string();
        let info = ResultInfo {
            id: id.clone(),
            columns: rs.columns.clone(),
            row_count: rs.rows.len(),
            truncated: rs.truncated,
            documents: rs.docs.is_some(),
        };
        self.map.lock().unwrap().insert(id, (window.to_string(), Arc::new(RwLock::new(rs))));
        info
    }

    pub fn get(&self, id: &str) -> Result<Shared, String> {
        self.map
            .lock()
            .unwrap()
            .get(id)
            .map(|(_, rs)| rs.clone())
            .ok_or_else(|| "This result is no longer available. Run the query again.".to_string())
    }

    pub fn free(&self, ids: &[String]) {
        let mut map = self.map.lock().unwrap();
        for id in ids {
            map.remove(id);
        }
    }

    pub fn free_window(&self, window: &str) {
        self.map.lock().unwrap().retain(|_, (w, _)| w != window);
    }
}

// ---------- copy & export ----------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// Tab-separated, no header: what Ctrl+C puts on the clipboard (pastes into Excel).
    Tsv,
    TsvHeader,
    Csv,
    CsvHeader,
    Json,
    Markdown,
    Insert,
    Xlsx,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    Mysql,
    Sqlite,
}

pub struct Selection<'a> {
    pub rows: Vec<&'a [Cell]>,
    pub cols: Vec<usize>,
    /// The rows' original indices (for their documents).
    pub idx: Vec<u32>,
}

impl ResultSet {
    pub fn select(&self, ranges: Option<&[[u32; 2]]>, cols: Option<&[u32]>) -> Selection<'_> {
        let idx = match ranges {
            Some(r) => self.view_indices(r),
            None => self.view.clone(),
        };
        let rows = idx.iter().map(|&i| &*self.rows[i as usize]).collect();
        let cols = match cols {
            Some(c) => c.iter().map(|&c| c as usize).filter(|&c| c < self.columns.len()).collect(),
            None => (0..self.columns.len()).collect(),
        };
        Selection { rows, cols, idx }
    }

    /// The selected rows' documents as relaxed Extended JSON; when only some columns are
    /// selected, only those top-level fields.
    fn selected_documents(&self, sel: &Selection) -> Option<Vec<Document>> {
        let docs = self.docs.as_ref()?;
        let all = sel.cols.len() == self.columns.len();
        let fields: Vec<&str> = sel.cols.iter().map(|&c| self.columns[c].name.as_str()).collect();
        Some(
            sel.idx
                .iter()
                .filter_map(|&i| Document::try_from(docs.get(i as usize)?.as_ref()).ok())
                .map(|d| if all { d } else { d.into_iter().filter(|(k, _)| fields.contains(&k.as_str())).collect() })
                .collect(),
        )
    }
}

fn cell<'a>(row: &'a [Cell], col: usize) -> Option<&'a str> {
    row.get(col).and_then(|c| c.as_deref())
}

pub fn to_text(rs: &ResultSet, sel: &Selection, format: Format, dialect: Dialect, table: Option<&str>) -> Result<String, String> {
    let names: Vec<&str> = sel.cols.iter().map(|&c| rs.columns[c].name.as_str()).collect();
    match format {
        Format::Tsv | Format::TsvHeader | Format::Csv | Format::CsvHeader => {
            let delimiter = if matches!(format, Format::Tsv | Format::TsvHeader) { b'\t' } else { b',' };
            let header = matches!(format, Format::TsvHeader | Format::CsvHeader);
            let mut w = csv::WriterBuilder::new()
                .delimiter(delimiter)
                .terminator(csv::Terminator::CRLF)
                .quote_style(csv::QuoteStyle::Necessary)
                .from_writer(Vec::new());
            if header {
                w.write_record(&names).map_err(|e| e.to_string())?;
            }
            for row in &sel.rows {
                w.write_record(sel.cols.iter().map(|&c| cell(row, c).unwrap_or(""))).map_err(|e| e.to_string())?;
            }
            let bytes = w.into_inner().map_err(|e| e.to_string())?;
            let mut text = String::from_utf8(bytes).map_err(|e| e.to_string())?;
            // A single value copies without the trailing line break.
            if sel.rows.len() == 1 && !header && text.ends_with("\r\n") {
                text.truncate(text.len() - 2);
            }
            Ok(text)
        }
        // MongoDB documents: relaxed Extended JSON with their nesting and types (mongoimport
        // --jsonArray reads it back).
        Format::Json if rs.has_documents() => {
            let docs = rs.selected_documents(sel).unwrap_or_default();
            let body: Vec<String> = docs
                .iter()
                .map(|d| super::mongo_doc::relaxed_json(d, true).lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n"))
                .collect();
            Ok(if body.is_empty() { "[]".into() } else { format!("[\n{}\n]", body.join(",\n")) })
        }
        Format::Json => {
            let mut out = String::from("[\n");
            for (r, row) in sel.rows.iter().enumerate() {
                out.push_str("  {");
                for (i, &c) in sel.cols.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&serde_json::to_string(&rs.columns[c].name).unwrap());
                    out.push_str(": ");
                    out.push_str(&json_value(&rs.columns[c], cell(row, c)));
                }
                out.push('}');
                if r + 1 < sel.rows.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push(']');
            Ok(out)
        }
        Format::Markdown => {
            let esc = |s: &str| s.replace('|', "\\|").replace("\r\n", "<br>").replace('\n', "<br>");
            let mut out = format!("| {} |\n", names.iter().map(|n| esc(n)).collect::<Vec<_>>().join(" | "));
            out.push_str(&format!(
                "| {} |\n",
                sel.cols.iter().map(|&c| if rs.columns[c].numeric { "---:" } else { "---" }).collect::<Vec<_>>().join(" | ")
            ));
            for row in &sel.rows {
                out.push_str(&format!(
                    "| {} |\n",
                    sel.cols.iter().map(|&c| esc(cell(row, c).unwrap_or("NULL"))).collect::<Vec<_>>().join(" | ")
                ));
            }
            Ok(out)
        }
        Format::Insert => {
            let table = table
                .map(str::to_string)
                .or_else(|| sel.cols.first().and_then(|&c| rs.columns[c].table.clone()))
                .unwrap_or_else(|| "table_name".into());
            let q = |s: &str| match dialect {
                Dialect::Mysql => format!("`{}`", s.replace('`', "``")),
                Dialect::Sqlite => format!("\"{}\"", s.replace('"', "\"\"")),
            };
            let cols = names.iter().map(|n| q(n)).collect::<Vec<_>>().join(", ");
            let mut out = String::new();
            for row in &sel.rows {
                let values = sel
                    .cols
                    .iter()
                    .map(|&c| sql_literal(&rs.columns[c], cell(row, c), dialect))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("INSERT INTO {} ({cols}) VALUES ({values});\n", q(&table)));
            }
            Ok(out)
        }
        Format::Xlsx => Err("Excel files can only be exported, not copied.".into()),
    }
}

fn is_plain_number(s: &str) -> bool {
    let t = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = t.split_once('.').unwrap_or((t, ""));
    !int.is_empty()
        && int.bytes().all(|c| c.is_ascii_digit())
        && frac.bytes().all(|c| c.is_ascii_digit())
        && (int == "0" || !int.starts_with('0'))
}

fn json_value(col: &Column, v: Option<&str>) -> String {
    match v {
        None => "null".into(),
        // Numbers are written exactly as the server sent them (no float rounding).
        Some(s) if col.numeric && is_plain_number(s) => s.to_string(),
        Some(s) if col.type_name == "json" && serde_json::from_str::<serde::de::IgnoredAny>(s).is_ok() => s.to_string(),
        Some(s) => serde_json::to_string(s).unwrap(),
    }
}

fn sql_literal(col: &Column, v: Option<&str>, dialect: Dialect) -> String {
    match v {
        None => "NULL".into(),
        Some(s) if col.numeric && is_plain_number(s) => s.to_string(),
        Some(s) if col.binary && s.starts_with("0x") => match dialect {
            Dialect::Mysql => s.to_string(),
            Dialect::Sqlite => format!("X'{}'", &s[2..]),
        },
        Some(s) => {
            let escaped = match dialect {
                // MySQL treats backslash as an escape character by default.
                Dialect::Mysql => s.replace('\\', "\\\\").replace('\'', "''"),
                Dialect::Sqlite => s.replace('\'', "''"),
            };
            format!("'{escaped}'")
        }
    }
}

/// Rows as multi-row INSERT statements (`rows_per_statement` rows each), for SQL dumps.
/// Only the columns in `sel.cols` are written (dumps leave out generated columns).
pub fn insert_statements(rs: &ResultSet, sel: &Selection<'_>, dialect: Dialect, table: &str, rows_per_statement: usize) -> String {
    let q = |s: &str| match dialect {
        Dialect::Mysql => format!("`{}`", s.replace('`', "``")),
        Dialect::Sqlite => format!("\"{}\"", s.replace('"', "\"\"")),
    };
    let cols = sel.cols.iter().map(|&c| q(&rs.columns[c].name)).collect::<Vec<_>>().join(", ");
    let mut out = String::new();
    for chunk in sel.rows.chunks(rows_per_statement.max(1)) {
        out.push_str(&format!("INSERT INTO {} ({cols}) VALUES
", q(table)));
        for (i, row) in chunk.iter().enumerate() {
            let values = sel.cols.iter().map(|&c| sql_literal(&rs.columns[c], cell(row, c), dialect)).collect::<Vec<_>>().join(", ");
            out.push_str(&format!("({values}){}
", if i + 1 == chunk.len() { ";" } else { "," }));
        }
    }
    out
}

pub fn export_file(
    rs: &ResultSet,
    path: &std::path::Path,
    format: Format,
    dialect: Dialect,
    table: Option<&str>,
) -> Result<usize, String> {
    let sel = rs.select(None, None);
    let count = sel.rows.len();
    if format == Format::Xlsx {
        let mut workbook = rust_xlsxwriter::Workbook::new();
        let sheet = workbook.add_worksheet();
        let bold = rust_xlsxwriter::Format::new().set_bold();
        for (i, &c) in sel.cols.iter().enumerate() {
            sheet.write_string_with_format(0, i as u16, &rs.columns[c].name, &bold).map_err(|e| e.to_string())?;
        }
        for (r, row) in sel.rows.iter().enumerate() {
            let r = (r + 1) as u32;
            for (i, &c) in sel.cols.iter().enumerate() {
                let Some(v) = cell(row, c) else { continue };
                // Excel keeps 15 significant digits; longer numbers stay text so they aren't rounded.
                let digits = v.bytes().filter(|b| b.is_ascii_digit()).count();
                if rs.columns[c].numeric && is_plain_number(v) && digits <= 15 {
                    if let Ok(n) = v.parse::<f64>() {
                        sheet.write_number(r, i as u16, n).map_err(|e| e.to_string())?;
                        continue;
                    }
                }
                sheet.write_string(r, i as u16, v).map_err(|e| e.to_string())?;
            }
        }
        sheet.set_freeze_panes(1, 0).map_err(|e| e.to_string())?;
        workbook.save(path).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
        return Ok(count);
    }
    let format = match format {
        Format::Tsv => Format::TsvHeader,
        other => other,
    };
    let text = to_text(rs, &sel, format, dialect, table)?;
    // A UTF-8 BOM lets Excel open CSV files with non-ASCII text correctly.
    let bom = if matches!(format, Format::Csv | Format::CsvHeader | Format::TsvHeader) { "\u{feff}" } else { "" };
    std::fs::write(path, format!("{bom}{text}")).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str, numeric: bool) -> Column {
        Column { name: name.into(), type_name: if numeric { "int".into() } else { "varchar".into() }, table: Some("t".into()), numeric, binary: false, primary_key: false }
    }

    fn rs() -> ResultSet {
        let rows: Vec<Box<[Cell]>> = vec![
            vec![Some("10".into()), Some("b|x".into())].into_boxed_slice(),
            vec![Some("9".into()), None].into_boxed_slice(),
            vec![Some("123456789012345678901".into()), Some("O'Brien \\ \"q\"".into())].into_boxed_slice(),
            vec![Some("-3.50".into()), Some("tab\there".into())].into_boxed_slice(),
        ];
        ResultSet::new(vec![col("id", true), col("name", false)], rows, false)
    }

    #[test]
    fn numeric_sort_is_exact() {
        let mut r = rs();
        r.set_sort(Some((0, false)));
        let ids: Vec<_> = r.rows(0, 10).iter().map(|row| row[0].unwrap().to_string()).collect();
        assert_eq!(ids, vec!["-3.50", "9", "10", "123456789012345678901"]);
        r.set_sort(Some((1, false)));
        assert_eq!(r.rows(0, 1)[0][1], None, "NULL sorts first");
    }

    #[test]
    fn rows_as_json_objects() {
        let mut r = ResultSet::new(
            vec![col("id", true), col("name", false), col("id", true), Column { type_name: "json".into(), ..col("meta", false) }],
            vec![
                vec![Some("123456789012345678901".into()), Some("O'Brien".into()), Some("7".into()), Some("{\"a\":[1,2]}".into())].into_boxed_slice(),
                vec![Some("2".into()), None, None, Some("not json".into())].into_boxed_slice(),
            ],
            false,
        );
        let objs = r.objects(0, 10);
        assert_eq!(objs.len(), 2);
        // Exact numbers, a repeated column name kept apart, JSON columns nested, NULL as null.
        assert_eq!(objs[0]["id"], serde_json::json!({ "$number": "123456789012345678901" }));
        assert_eq!(objs[0]["id (2)"], serde_json::json!({ "$number": "7" }));
        assert_eq!(objs[0]["meta"], serde_json::json!({ "a": [1, 2] }));
        assert_eq!(objs[1]["name"], serde_json::Value::Null);
        assert_eq!(objs[1]["meta"], serde_json::json!("not json"));
        assert_eq!(r.row_json(0).unwrap(), "{\n  \"id\": 123456789012345678901,\n  \"name\": \"O'Brien\",\n  \"id (2)\": 7,\n  \"meta\": {\"a\":[1,2]}\n}");
        // The view's order and search apply.
        r.set_filter(Some("2".into()));
        assert_eq!(r.objects(0, 10).len(), 2);
        r.set_sort(Some((0, false)));
        assert_eq!(r.objects(0, 1)[0]["id"], serde_json::json!({ "$number": "2" }));
    }

    #[test]
    fn filter_counts() {
        let mut r = rs();
        assert_eq!(r.set_filter(Some("o'b".into())), 1);
        assert_eq!(r.set_filter(None), 4);
    }

    #[test]
    fn formats() {
        let r = rs();
        let sel = r.select(Some(&[[0, 4]]), None);
        let json = to_text(&r, &sel, Format::Json, Dialect::Mysql, None).unwrap();
        assert!(json.contains("\"id\": 123456789012345678901"), "{json}");
        assert!(json.contains("\"id\": -3.50"), "trailing zero kept: {json}");
        assert!(json.contains("\"name\": null"));
        let ins = to_text(&r, &sel, Format::Insert, Dialect::Mysql, None).unwrap();
        assert!(ins.contains("INSERT INTO `t` (`id`, `name`) VALUES (123456789012345678901, 'O''Brien \\\\ \"q\"');"), "{ins}");
        let ins_lite = to_text(&r, &sel, Format::Insert, Dialect::Sqlite, Some("people")).unwrap();
        assert!(ins_lite.contains("INSERT INTO \"people\""));
        assert!(ins_lite.contains("'O''Brien \\ \"q\"'"), "{ins_lite}");
        let md = to_text(&r, &sel, Format::Markdown, Dialect::Mysql, None).unwrap();
        assert!(md.contains("| 10 | b\\|x |"), "{md}");
        let tsv = to_text(&r, &sel, Format::Tsv, Dialect::Mysql, None).unwrap();
        assert!(tsv.contains("\"tab\there\""), "tabs inside values are quoted: {tsv}");
        let one = r.select(Some(&[[1, 2]]), Some(&[0]));
        assert_eq!(to_text(&r, &one, Format::Tsv, Dialect::Mysql, None).unwrap(), "9");
    }
}
