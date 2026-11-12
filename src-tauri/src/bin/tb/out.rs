//! Printing results: a boxed table in a terminal, or CSV / TSV / JSON / Markdown / SQL (the
//! same writers as the app's Copy and Export).

use crate::cli::Fmt;
use crate::term;
use std::io::Write;
use tableboat_core::db::results::{self, Cell, Column, Format, ResultSet};

/// The output format: the one asked for, else a table in a terminal and TSV in a pipe.
pub fn pick(fmt: Option<Fmt>) -> Fmt {
    fmt.unwrap_or(if term::stdout_is_terminal() { Fmt::Table } else { Fmt::Tsv })
}

/// A simple list (connections, tables…) as a result set, so every format works for it.
pub fn list(headers: &[&str], rows: Vec<Vec<Option<String>>>, numeric: &[usize]) -> ResultSet {
    let columns = headers
        .iter()
        .enumerate()
        .map(|(i, h)| Column {
            name: h.to_string(),
            type_name: String::new(),
            table: None,
            numeric: numeric.contains(&i),
            binary: false,
            primary_key: false,
        })
        .collect();
    let rows = rows.into_iter().map(|r| r.into_iter().map(|c| c.map(Box::<str>::from)).collect::<Vec<Cell>>().into_boxed_slice()).collect();
    ResultSet::new(columns, rows, false)
}

pub fn print(rs: &ResultSet, fmt: Fmt, dialect: results::Dialect, table: Option<&str>, wide: bool) -> Result<(), String> {
    let text = match fmt {
        Fmt::Table => render_table(rs, wide),
        Fmt::Xlsx => return Err("Excel output only works with `tb export … -o file.xlsx`.".into()),
        other => {
            let format = match other {
                Fmt::Csv => Format::CsvHeader,
                Fmt::Tsv => Format::TsvHeader,
                Fmt::Json => Format::Json,
                Fmt::Md => Format::Markdown,
                Fmt::Sql => Format::Insert,
                _ => unreachable!(),
            };
            let sel = rs.select(None, None);
            let mut t = results::to_text(rs, &sel, format, dialect, table)?;
            if !t.ends_with('\n') {
                t.push('\n');
            }
            t
        }
    };
    let mut out = std::io::stdout().lock();
    // A closed pipe (e.g. `| head`) is not an error worth reporting.
    let _ = out.write_all(text.as_bytes());
    let _ = out.flush();
    Ok(())
}

fn display(cell: Option<&str>) -> String {
    match cell {
        None => "NULL".into(),
        Some(v) => v.replace("\r\n", "⏎").replace(['\n', '\r'], "⏎").replace('\t', " "),
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

fn cut(s: &str, max: usize) -> String {
    if width(s) <= max { s.to_string() } else { format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>()) }
}

/// +----+-------+
/// | id | name  |
/// +----+-------+
fn render_table(rs: &ResultSet, wide: bool) -> String {
    let sel = rs.select(None, None);
    let cap = if wide { usize::MAX } else { 60 };
    let cols: Vec<usize> = sel.cols.clone();
    let mut widths: Vec<usize> = cols.iter().map(|&c| width(&rs.columns[c].name).min(cap)).collect();
    let cells: Vec<Vec<(String, bool)>> = sel
        .rows
        .iter()
        .map(|row| {
            cols.iter()
                .map(|&c| {
                    let v = row.get(c).and_then(|x| x.as_deref());
                    (cut(&display(v), cap), v.is_none())
                })
                .collect()
        })
        .collect();
    for row in &cells {
        for (i, (v, _)) in row.iter().enumerate() {
            widths[i] = widths[i].max(width(v));
        }
    }
    let line = format!("+{}+\n", widths.iter().map(|w| "-".repeat(w + 2)).collect::<Vec<_>>().join("+"));
    let mut out = String::new();
    out.push_str(&line);
    out.push('|');
    for (i, &c) in cols.iter().enumerate() {
        let name = cut(&rs.columns[c].name, cap);
        out.push_str(&format!(" {}{} |", name, " ".repeat(widths[i] - width(&name))));
    }
    out.push('\n');
    out.push_str(&line);
    let colour = term::stdout_is_terminal();
    for row in &cells {
        out.push('|');
        for (i, (v, null)) in row.iter().enumerate() {
            let pad = " ".repeat(widths[i] - width(v));
            let shown = if *null && colour { term::dim(v) } else { v.clone() };
            if rs.columns[cols[i]].numeric && !null {
                out.push_str(&format!(" {pad}{shown} |"));
            } else {
                out.push_str(&format!(" {shown}{pad} |"));
            }
        }
        out.push('\n');
    }
    if !cells.is_empty() {
        out.push_str(&line);
    }
    out
}
