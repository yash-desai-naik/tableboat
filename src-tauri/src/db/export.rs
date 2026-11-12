//! Exports shared by the `tb` command line and Claude's app tools: a table, collection or query
//! to a file, and a whole database as a SQL script (MySQL, SQLite) or a folder with one JSON file
//! per collection (MongoDB).
//!
//! Dumps read each table in chunks, so memory stays small however big the database is.

use super::meta::{self, ObjectKind, quote_mysql, quote_sqlite};
use super::query::{self, RunOptions, Runs};
use super::results::{self, Format, ResultSet};
use super::{Backend, SessionHandle};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Rows read per query while dumping.
const CHUNK: usize = 5_000;
/// Rows per INSERT statement in a dump.
const ROWS_PER_INSERT: usize = 200;

/// The export format for a file name (`asked` overrides): csv, tsv, json, md, sql, xlsx.
pub fn format_for(path: &Path, asked: Option<&str>) -> Result<Format, String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let name = asked.map(str::to_ascii_lowercase).or(ext);
    Ok(match name.as_deref() {
        Some("csv") => Format::CsvHeader,
        Some("tsv" | "txt") => Format::TsvHeader,
        Some("json") => Format::Json,
        Some("md" | "markdown") => Format::Markdown,
        Some("sql") => Format::Insert,
        Some("xlsx") => Format::Xlsx,
        _ => {
            return Err(format!(
                "Can't tell the format from \"{}\": use .csv, .tsv, .json, .xlsx, .md or .sql.",
                path.display()
            ));
        }
    })
}

/// The dialect INSERT statements are written in.
pub fn dialect(session: &SessionHandle) -> results::Dialect {
    match session.backend {
        Backend::Sqlite(_) => results::Dialect::Sqlite,
        _ => results::Dialect::Mysql,
    }
}

/// The statement that reads a whole table or collection (`limit` rows at most).
pub fn table_query(session: &SessionHandle, database: Option<&str>, table: &str, limit: Option<usize>) -> String {
    let limit = limit.map(|n| n.to_string());
    match session.backend {
        Backend::Mongo(_) => {
            let base = format!("db.getCollection({}).find({{}})", serde_json::to_string(table).unwrap_or_default());
            limit.map(|n| format!("{base}.limit({n})")).unwrap_or(base)
        }
        Backend::Sqlite(_) => {
            let qual = database.map(|d| format!("{}.", quote_sqlite(d))).unwrap_or_default();
            format!("SELECT * FROM {qual}{}{}", quote_sqlite(table), limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default())
        }
        Backend::Mysql(_) => {
            let qual = database.map(|d| format!("{}.", quote_mysql(d))).unwrap_or_default();
            format!("SELECT * FROM {qual}{}{}", quote_mysql(table), limit.map(|n| format!(" LIMIT {n}")).unwrap_or_default())
        }
    }
}

async fn read(session: &SessionHandle, runs: &Runs, database: Option<&str>, sql: String, max_rows: usize) -> Result<ResultSet, String> {
    let opts = RunOptions {
        // SQLite statements name their schema themselves.
        database: database.filter(|_| !matches!(session.backend, Backend::Sqlite(_))).map(str::to_string),
        max_rows,
        use_pool: true,
        stop_on_error: true,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let mut outcomes = query::run(session, runs, &id, vec![sql], opts).await?;
    let o = outcomes.pop().ok_or("Nothing ran.")?;
    if let Some(e) = o.error {
        return Err(e);
    }
    o.result.ok_or_else(|| "That statement returned no rows to export.".to_string())
}

/// Run `sql` and write all its rows (up to `max_rows`) to `path`. `table` names the table in
/// INSERT statements. Returns the number of rows written.
pub async fn query_to_file(
    session: &SessionHandle,
    runs: &Runs,
    database: Option<&str>,
    sql: String,
    table: Option<&str>,
    path: &Path,
    format: Format,
    max_rows: usize,
) -> Result<usize, String> {
    let rs = read(session, runs, database, sql, max_rows).await?;
    results::export_file(&rs, path, format, dialect(session), table)
}

#[derive(Default)]
pub struct DumpOptions {
    /// Only these tables (or collections); `None` = all.
    pub tables: Option<Vec<String>>,
    /// Include the rows (otherwise only the structure).
    pub data: bool,
}

#[derive(Debug, Default)]
pub struct DumpStats {
    /// MongoDB: these are collections.
    pub collections: bool,
    pub tables: usize,
    pub views: usize,
    pub routines: usize,
    pub rows: u64,
    /// The file written (SQL), or the folder with one file per collection (MongoDB).
    pub path: PathBuf,
}

impl DumpStats {
    pub fn describe(&self) -> String {
        let what = if self.collections { "collection" } else { "table" };
        let mut parts = vec![format!("{} {what}{}", self.tables, if self.tables == 1 { "" } else { "s" })];
        if self.views > 0 {
            parts.push(format!("{} view{}", self.views, if self.views == 1 { "" } else { "s" }));
        }
        if self.routines > 0 {
            parts.push(format!("{} routine/trigger{}", self.routines, if self.routines == 1 { "" } else { "s" }));
        }
        let unit = if self.collections { "document" } else { "row" };
        parts.push(format!("{} {unit}{}", self.rows, if self.rows == 1 { "" } else { "s" }));
        parts.join(", ")
    }
}

/// Dump a whole database: a SQL script (MySQL, SQLite) or, for MongoDB, a folder at `path` with
/// one JSON file per collection. A failed dump removes what it had written.
pub async fn dump(session: &SessionHandle, runs: &Runs, database: &str, path: &Path, opts: &DumpOptions) -> Result<DumpStats, String> {
    let result = match session.backend {
        Backend::Mongo(_) => dump_mongo(session, runs, database, path, opts).await,
        _ => dump_sql(session, runs, database, path, opts).await,
    };
    if result.is_err() && !matches!(session.backend, Backend::Mongo(_)) {
        let _ = std::fs::remove_file(path);
    }
    result
}

fn wanted(opts: &DumpOptions, name: &str) -> bool {
    opts.tables.as_ref().is_none_or(|list| list.iter().any(|t| t == name))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("Could not write {}: {e}", path.display())
}

async fn dump_sql(session: &SessionHandle, runs: &Runs, database: &str, path: &Path, opts: &DumpOptions) -> Result<DumpStats, String> {
    let mysql = matches!(session.backend, Backend::Mysql(_));
    let objects = meta::objects(session, database, true).await?;
    if let Some(list) = &opts.tables {
        if let Some(missing) = list.iter().find(|t| !objects.iter().any(|o| &o.name == *t)) {
            return Err(format!("There's no table \"{missing}\" in {database}."));
        }
    }
    let file = std::fs::File::create(path).map_err(io(path))?;
    let mut out = std::io::BufWriter::new(file);
    let mut stats = DumpStats { path: path.to_path_buf(), ..Default::default() };
    let q = |s: &str| if mysql { quote_mysql(s) } else { quote_sqlite(s) };

    writeln!(
        out,
        "-- Tableboat dump of {} ({} {}), {}\n-- {}\n",
        q(database),
        session.info.product,
        session.info.version,
        crate::paths::utc_stamp(crate::paths::now_secs()),
        if opts.data { "Structure and data." } else { "Structure only." }
    )
    .map_err(io(path))?;
    if mysql {
        writeln!(out, "SET NAMES utf8mb4;\nSET FOREIGN_KEY_CHECKS = 0;\nSET UNIQUE_CHECKS = 0;\n").map_err(io(path))?;
    } else {
        writeln!(out, "PRAGMA foreign_keys = OFF;\nBEGIN;\n").map_err(io(path))?;
    }

    // MySQL generated columns can't be inserted into: leave them out of the INSERTs.
    let generated: Vec<(String, String)> = if mysql && opts.data {
        let sql = format!(
            "SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = '{}' AND EXTRA LIKE '%GENERATED%'",
            database.replace('\\', "\\\\").replace('\'', "''")
        );
        let rs = read(session, runs, None, sql, 100_000).await.unwrap_or_else(|_| ResultSet::new(vec![], vec![], false));
        rs.select(None, None)
            .rows
            .iter()
            .filter_map(|r| Some((r.first()?.as_deref()?.to_string(), r.get(1)?.as_deref()?.to_string())))
            .collect()
    } else {
        vec![]
    };

    for t in objects.iter().filter(|o| o.kind == ObjectKind::Table && wanted(opts, &o.name)) {
        let ddl = meta::ddl(session, database, ObjectKind::Table, &t.name).await?;
        writeln!(out, "-- Table {}\nDROP TABLE IF EXISTS {};\n{}\n", q(&t.name), q(&t.name), ddl.trim_end()).map_err(io(path))?;
        stats.tables += 1;
        if !opts.data {
            continue;
        }
        let mut offset = 0usize;
        loop {
            let sql = format!("{} LIMIT {CHUNK} OFFSET {offset}", table_query(session, Some(database), &t.name, None));
            let rs = read(session, runs, Some(database), sql, CHUNK).await.map_err(|e| format!("Reading {}: {e}", t.name))?;
            let cols: Vec<u32> = (0..rs.columns.len())
                .filter(|&c| !generated.iter().any(|(tab, col)| tab == &t.name && col == &rs.columns[c].name))
                .map(|c| c as u32)
                .collect();
            let sel = rs.select(None, Some(&cols));
            let n = sel.rows.len();
            if n > 0 {
                out.write_all(results::insert_statements(&rs, &sel, dialect(session), &t.name, ROWS_PER_INSERT).as_bytes()).map_err(io(path))?;
            }
            stats.rows += n as u64;
            offset += n;
            if n < CHUNK {
                break;
            }
        }
        writeln!(out).map_err(io(path))?;
    }

    for v in objects.iter().filter(|o| o.kind == ObjectKind::View && wanted(opts, &o.name)) {
        let ddl = meta::ddl(session, database, ObjectKind::View, &v.name).await?;
        writeln!(out, "-- View {}\nDROP VIEW IF EXISTS {};\n{}\n", q(&v.name), q(&v.name), ddl.trim_end()).map_err(io(path))?;
        stats.views += 1;
    }

    if mysql && opts.tables.is_none() {
        // Bodies contain semicolons: DELIMITER blocks, as the mysql client (and `tb query`) read them.
        for kind in [ObjectKind::Function, ObjectKind::Procedure, ObjectKind::Trigger] {
            for o in objects.iter().filter(|o| o.kind == kind) {
                let Ok(ddl) = meta::ddl(session, database, kind, &o.name).await else { continue };
                let word = match kind {
                    ObjectKind::Function => "FUNCTION",
                    ObjectKind::Procedure => "PROCEDURE",
                    _ => "TRIGGER",
                };
                let body = ddl.trim_end().trim_end_matches(';');
                writeln!(out, "-- {} {}\nDROP {word} IF EXISTS {};\nDELIMITER ;;\n{body} ;;\nDELIMITER ;\n", word.to_lowercase(), q(&o.name), q(&o.name))
                    .map_err(io(path))?;
                stats.routines += 1;
            }
        }
    } else if !mysql {
        // SQLite keeps indexes and triggers as separate entries.
        let names = objects.iter().filter(|o| o.kind == ObjectKind::Table && wanted(opts, &o.name)).map(|o| o.name.clone()).collect::<Vec<_>>();
        let sql = format!("SELECT tbl_name, sql FROM {}.sqlite_master WHERE type IN ('index', 'trigger') AND sql IS NOT NULL", quote_sqlite(database));
        let rs = read(session, runs, None, sql, 100_000).await?;
        for r in rs.select(None, None).rows {
            let (Some(table), Some(sql)) = (r.first().and_then(|c| c.as_deref()), r.get(1).and_then(|c| c.as_deref())) else { continue };
            if names.iter().any(|n| n == table) {
                writeln!(out, "{};\n", sql.trim_end().trim_end_matches(';')).map_err(io(path))?;
                stats.routines += 1;
            }
        }
    }

    if mysql {
        writeln!(out, "SET UNIQUE_CHECKS = 1;\nSET FOREIGN_KEY_CHECKS = 1;").map_err(io(path))?;
    } else {
        writeln!(out, "COMMIT;\nPRAGMA foreign_keys = ON;").map_err(io(path))?;
    }
    out.flush().map_err(io(path))?;
    Ok(stats)
}

/// A file name that Windows accepts for a collection name.
fn safe_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    if s.trim_matches('.').is_empty() { "_".into() } else { s }
}

/// One file per collection, one document per line in canonical Extended JSON (`mongoexport
/// --jsonFormat=canonical`; `mongoimport` reads it): every type and all nesting kept exactly.
/// Read straight from a cursor, so memory stays small.
async fn dump_mongo(session: &SessionHandle, _runs: &Runs, database: &str, folder: &Path, opts: &DumpOptions) -> Result<DumpStats, String> {
    use futures_util::TryStreamExt;
    let Backend::Mongo(b) = &session.backend else { return Err("Not a MongoDB connection.".into()) };
    let objects = meta::objects(session, database, true).await?;
    let collections: Vec<_> = objects.iter().filter(|o| o.kind == ObjectKind::Table && wanted(opts, &o.name)).collect();
    if let Some(list) = &opts.tables {
        if let Some(missing) = list.iter().find(|t| !collections.iter().any(|o| &o.name == *t)) {
            return Err(format!("There's no collection \"{missing}\" in {database}."));
        }
    }
    if folder.is_file() {
        return Err(format!("{} is a file; MongoDB databases are exported to a folder (one JSON file per collection).", folder.display()));
    }
    std::fs::create_dir_all(folder).map_err(io(folder))?;
    let db = b.client.database(database);
    let mut stats = DumpStats { path: folder.to_path_buf(), collections: true, ..Default::default() };
    for c in collections {
        let path = folder.join(format!("{}.json", safe_file_name(&c.name)));
        let file = std::fs::File::create(&path).map_err(io(&path))?;
        let mut out = std::io::BufWriter::new(file);
        if opts.data {
            let coll = db.collection::<mongodb::bson::Document>(&c.name);
            let mut cursor = coll.find(mongodb::bson::doc! {}).await.map_err(|e| format!("Reading {}: {e}", c.name))?;
            while let Some(d) = cursor.try_next().await.map_err(|e| format!("Reading {}: {e}", c.name))? {
                writeln!(out, "{}", super::mongo_doc::canonical_json(&d)).map_err(io(&path))?;
                stats.rows += 1;
            }
        }
        out.flush().map_err(io(&path))?;
        stats.tables += 1;
    }
    Ok(stats)
}
