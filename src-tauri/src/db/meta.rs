//! Database objects for the sidebar and Ctrl+P: databases, tables, views, functions,
//! procedures and triggers, plus their definitions (DDL). Lists are cached per session
//! and only reloaded on Refresh.

use super::{Backend, SessionHandle};
use mysql_async::Row;
use mysql_async::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Table,
    View,
    Function,
    Procedure,
    Trigger,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DbObject {
    pub database: String,
    pub name: String,
    pub kind: ObjectKind,
    /// For triggers: the table it belongs to.
    pub parent: Option<String>,
    /// Estimated rows (MySQL statistics; may be approximate).
    pub rows: Option<u64>,
    pub comment: Option<String>,
}

#[derive(Default)]
pub struct MetaCache {
    databases: Option<Vec<String>>,
    objects: HashMap<String, Vec<DbObject>>,
    /// Every database's objects have been loaded (for Ctrl+P).
    all_loaded: bool,
}

/// Databases that are internal to the server and hidden from the lists.
const MYSQL_SYSTEM: [&str; 4] = ["information_schema", "mysql", "performance_schema", "sys"];

pub fn quote_mysql(ident: &str) -> String {
    format!("`{}`", ident.replace('`', "``"))
}

pub fn quote_sqlite(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

fn mysql_err(e: mysql_async::Error) -> String {
    match e {
        mysql_async::Error::Server(s) => format!("{} (MySQL error {})", s.message, s.code),
        other => other.to_string(),
    }
}

// ---------- loading ----------

async fn load_databases(backend: &Backend, fallback: Option<&str>) -> Result<Vec<String>, String> {
    match backend {
        Backend::Mongo(b) => super::mongo::databases(b, fallback).await,
        Backend::Mysql(b) => {
            let mut c = b.pool.get_conn().await.map_err(mysql_err)?;
            let names: Vec<String> = c.query("SHOW DATABASES").await.map_err(mysql_err)?;
            let (mut user, system): (Vec<String>, Vec<String>) =
                names.into_iter().partition(|n| !MYSQL_SYSTEM.contains(&n.to_ascii_lowercase().as_str()));
            user.sort_by_key(|n| n.to_lowercase());
            // System databases last, so they're reachable but out of the way.
            user.extend(system);
            Ok(user)
        }
        Backend::Sqlite(b) => {
            let b = b.clone();
            tokio::task::spawn_blocking(move || {
                let conn = b.conn.lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare("PRAGMA database_list").map_err(|e| e.to_string())?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(1))
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .filter(|n| n != "temp")
                    .collect::<Vec<_>>();
                Ok(names)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

/// Objects of one database, or of every non-system database when `database` is None.
async fn load_objects(backend: &Backend, database: Option<&str>) -> Result<Vec<DbObject>, String> {
    match backend {
        Backend::Mongo(b) => match database {
            Some(db) => super::mongo::objects(b, db).await,
            None => {
                // Ctrl+P across every database (system ones left out).
                let mut out = Vec::new();
                for db in super::mongo::databases(b, None).await.unwrap_or_default() {
                    if ["admin", "config", "local"].contains(&db.as_str()) {
                        continue;
                    }
                    if let Ok(list) = super::mongo::objects(b, &db).await {
                        out.extend(list);
                    }
                }
                Ok(out)
            }
        },
        Backend::Mysql(b) => {
            let mut c = b.pool.get_conn().await.map_err(mysql_err)?;
            let (filter, params): (String, Vec<String>) = match database {
                Some(db) => ("= ?".into(), vec![db.to_string()]),
                None => (
                    format!("NOT IN ({})", MYSQL_SYSTEM.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")),
                    vec![],
                ),
            };

            let mut out = Vec::new();

            let tables: Vec<(String, String, String, Option<u64>, Option<String>)> = c
                .exec(
                    format!(
                        "SELECT TABLE_SCHEMA, TABLE_NAME, TABLE_TYPE, TABLE_ROWS, TABLE_COMMENT \
                         FROM information_schema.TABLES WHERE TABLE_SCHEMA {filter}"
                    ),
                    params.clone(),
                )
                .await
                .map_err(mysql_err)?;
            for (db, name, ty, rows, comment) in tables {
                let is_view = ty.eq_ignore_ascii_case("VIEW");
                out.push(DbObject {
                    database: db,
                    name,
                    kind: if is_view { ObjectKind::View } else { ObjectKind::Table },
                    parent: None,
                    rows: if is_view { None } else { rows },
                    comment: comment.filter(|c| !c.is_empty() && !is_view),
                });
            }

            let routines: Vec<(String, String, String)> = c
                .exec(
                    format!(
                        "SELECT ROUTINE_SCHEMA, ROUTINE_NAME, ROUTINE_TYPE \
                         FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA {filter}"
                    ),
                    params.clone(),
                )
                .await
                .map_err(mysql_err)?;
            for (db, name, ty) in routines {
                out.push(DbObject {
                    database: db,
                    name,
                    kind: if ty.eq_ignore_ascii_case("PROCEDURE") { ObjectKind::Procedure } else { ObjectKind::Function },
                    parent: None,
                    rows: None,
                    comment: None,
                });
            }

            let triggers: Vec<(String, String, String, String, String)> = c
                .exec(
                    format!(
                        "SELECT TRIGGER_SCHEMA, TRIGGER_NAME, EVENT_OBJECT_TABLE, ACTION_TIMING, EVENT_MANIPULATION \
                         FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA {filter}"
                    ),
                    params,
                )
                .await
                .map_err(mysql_err)?;
            for (db, name, table, timing, event) in triggers {
                out.push(DbObject {
                    database: db,
                    name,
                    kind: ObjectKind::Trigger,
                    parent: Some(table),
                    rows: None,
                    comment: Some(format!("{} {}", timing, event).to_lowercase()),
                });
            }

            out.sort_by(|a, b| (a.database.to_lowercase(), a.name.to_lowercase()).cmp(&(b.database.to_lowercase(), b.name.to_lowercase())));
            Ok(out)
        }
        Backend::Sqlite(b) => {
            let b = b.clone();
            let databases = match database {
                Some(db) => vec![db.to_string()],
                None => load_databases(&Backend::Sqlite(b.clone()), None).await?,
            };
            tokio::task::spawn_blocking(move || {
                let conn = b.conn.lock().map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                for db in databases {
                    let sql = format!(
                        "SELECT type, name, tbl_name FROM {}.sqlite_master \
                         WHERE type IN ('table','view','trigger') AND name NOT LIKE 'sqlite_%' \
                         ORDER BY name COLLATE NOCASE",
                        quote_sqlite(&db)
                    );
                    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                        .map_err(|e| e.to_string())?;
                    for row in rows.filter_map(Result::ok) {
                        let (ty, name, table) = row;
                        let kind = match ty.as_str() {
                            "view" => ObjectKind::View,
                            "trigger" => ObjectKind::Trigger,
                            _ => ObjectKind::Table,
                        };
                        out.push(DbObject {
                            database: db.clone(),
                            parent: (kind == ObjectKind::Trigger).then_some(table),
                            name,
                            kind,
                            rows: None,
                            comment: None,
                        });
                    }
                }
                Ok(out)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

// ---------- cached access ----------

pub async fn databases(session: &SessionHandle, refresh: bool) -> Result<Vec<String>, String> {
    let mut cache = session.meta.lock().await;
    if refresh || cache.databases.is_none() {
        let fallback = Some(session.connection.database.as_str()).filter(|d| !d.is_empty());
        cache.databases = Some(load_databases(&session.backend, fallback).await?);
    }
    Ok(cache.databases.clone().unwrap_or_default())
}

pub async fn objects(session: &SessionHandle, database: &str, refresh: bool) -> Result<Vec<DbObject>, String> {
    let mut cache = session.meta.lock().await;
    if refresh || !cache.objects.contains_key(database) {
        let list = load_objects(&session.backend, Some(database)).await?;
        cache.objects.insert(database.to_string(), list);
        if refresh {
            cache.all_loaded = false;
        }
    }
    Ok(cache.objects.get(database).cloned().unwrap_or_default())
}

/// Every object in every (non-system) database, loaded once in a single pass.
async fn all_objects(session: &SessionHandle) -> Result<Vec<DbObject>, String> {
    let mut cache = session.meta.lock().await;
    if !cache.all_loaded {
        let list = load_objects(&session.backend, None).await?;
        let mut by_db: HashMap<String, Vec<DbObject>> = HashMap::new();
        for o in list {
            by_db.entry(o.database.clone()).or_default().push(o);
        }
        // Keep lists already loaded for system databases the user opened explicitly.
        for (db, list) in by_db {
            cache.objects.insert(db, list);
        }
        cache.all_loaded = true;
    }
    let mut all: Vec<DbObject> = cache.objects.values().flatten().cloned().collect();
    all.sort_by(|a, b| (a.database.to_lowercase(), a.name.to_lowercase()).cmp(&(b.database.to_lowercase(), b.name.to_lowercase())));
    Ok(all)
}

pub async fn clear(session: &SessionHandle) {
    *session.meta.lock().await = MetaCache::default();
}

// ---------- definitions ----------

pub async fn ddl(session: &SessionHandle, database: &str, kind: ObjectKind, name: &str) -> Result<String, String> {
    match &session.backend {
        Backend::Mongo(b) => {
            let _ = kind;
            super::mongo::definition(b, database, name).await
        }
        Backend::Mysql(b) => {
            let target = format!("{}.{}", quote_mysql(database), quote_mysql(name));
            let (sql, column) = match kind {
                ObjectKind::Table => (format!("SHOW CREATE TABLE {target}"), 1),
                ObjectKind::View => (format!("SHOW CREATE VIEW {target}"), 1),
                ObjectKind::Function => (format!("SHOW CREATE FUNCTION {target}"), 2),
                ObjectKind::Procedure => (format!("SHOW CREATE PROCEDURE {target}"), 2),
                ObjectKind::Trigger => (format!("SHOW CREATE TRIGGER {target}"), 2),
            };
            let mut c = b.pool.get_conn().await.map_err(mysql_err)?;
            let row: Option<Row> = c.query_first(sql).await.map_err(mysql_err)?;
            let text = row
                .and_then(|r| r.get_opt::<Option<String>, usize>(column).and_then(Result::ok).flatten())
                .ok_or_else(|| {
                    format!("The server didn't return the definition of {name}. Your user may lack the privilege to see it.")
                })?;
            Ok(if text.trim_end().ends_with(';') { text } else { format!("{text};") })
        }
        Backend::Sqlite(b) => {
            let b = b.clone();
            let sql = format!("SELECT sql FROM {}.sqlite_master WHERE name = ?1", quote_sqlite(database));
            let name = name.to_string();
            tokio::task::spawn_blocking(move || {
                let conn = b.conn.lock().map_err(|e| e.to_string())?;
                let text: Option<String> = conn
                    .query_row(&sql, [&name], |r| r.get(0))
                    .map_err(|e| format!("Could not read the definition of {name}: {e}"))?;
                let text = text.unwrap_or_else(|| format!("-- {name} has no stored definition (built-in object)"));
                Ok(if text.trim_end().ends_with(';') { text } else { format!("{text};") })
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

// ---------- columns (autocomplete) ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    pub type_name: String,
}

/// Every table's columns in one database, in table order, for editor autocomplete.
pub async fn columns(session: &SessionHandle, database: &str) -> Result<HashMap<String, Vec<ColumnInfo>>, String> {
    let mut out: HashMap<String, Vec<ColumnInfo>> = HashMap::new();
    match &session.backend {
        // MongoDB has no fixed columns; the editor uses JavaScript, without schema completion.
        Backend::Mongo(_) => {}
        Backend::Mysql(b) => {
            let mut c = b.pool.get_conn().await.map_err(mysql_err)?;
            let rows: Vec<(String, String, String)> = c
                .exec(
                    "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION",
                    (database,),
                )
                .await
                .map_err(mysql_err)?;
            for (table, name, ty) in rows {
                out.entry(table).or_default().push(ColumnInfo { name, type_name: ty });
            }
        }
        Backend::Sqlite(b) => {
            let b = b.clone();
            let db = database.to_string();
            out = tokio::task::spawn_blocking(move || {
                let conn = b.conn.lock().map_err(|e| e.to_string())?;
                let mut out: HashMap<String, Vec<ColumnInfo>> = HashMap::new();
                let tables: Vec<String> = {
                    let mut stmt = conn
                        .prepare(&format!(
                            "SELECT name FROM {}.sqlite_master WHERE type IN ('table','view') AND name NOT LIKE 'sqlite_%'",
                            quote_sqlite(&db)
                        ))
                        .map_err(|e| e.to_string())?;
                    stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?.filter_map(Result::ok).collect()
                };
                for table in tables {
                    let sql = format!("PRAGMA {}.table_info({})", quote_sqlite(&db), quote_sqlite(&table));
                    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                    let cols: Vec<ColumnInfo> = stmt
                        .query_map([], |r| Ok(ColumnInfo { name: r.get(1)?, type_name: r.get::<_, String>(2)?.to_lowercase() }))
                        .map_err(|e| e.to_string())?
                        .filter_map(Result::ok)
                        .collect();
                    out.insert(table, cols);
                }
                Ok::<_, String>(out)
            })
            .await
            .map_err(|e| e.to_string())??;
        }
    }
    Ok(out)
}

// ---------- one table's columns (editing) ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableColumn {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
    pub auto_increment: bool,
    /// Computed by the server (VIRTUAL/STORED GENERATED): never written.
    pub generated: bool,
    /// Shown and edited as hex.
    pub binary: bool,
}

pub async fn table_columns(session: &SessionHandle, database: &str, table: &str) -> Result<Vec<TableColumn>, String> {
    match &session.backend {
        Backend::Mongo(b) => super::mongo::sample_fields(b, database, table, 50).await,
        Backend::Mysql(b) => {
            let mut c = b.pool.get_conn().await.map_err(mysql_err)?;
            let rows: Vec<(String, String, String, Option<String>, String, String, String)> = c
                .exec(
                    "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, COLUMN_KEY, EXTRA, DATA_TYPE \
                     FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
                    (database, table),
                )
                .await
                .map_err(mysql_err)?;
            if rows.is_empty() {
                return Err(format!("Table {database}.{table} was not found."));
            }
            Ok(rows
                .into_iter()
                .map(|(name, ty, nullable, default, key, extra, data_type)| {
                    let extra = extra.to_ascii_lowercase();
                    let dt = data_type.to_ascii_lowercase();
                    TableColumn {
                        name,
                        type_name: ty,
                        nullable: nullable.eq_ignore_ascii_case("YES"),
                        default,
                        primary_key: key.eq_ignore_ascii_case("PRI"),
                        auto_increment: extra.contains("auto_increment"),
                        generated: extra.contains("generated"),
                        binary: matches!(
                            dt.as_str(),
                            "binary" | "varbinary" | "tinyblob" | "blob" | "mediumblob" | "longblob" | "bit" | "geometry"
                                | "point" | "linestring" | "polygon" | "multipoint" | "multilinestring" | "multipolygon"
                                | "geometrycollection"
                        ),
                    }
                })
                .collect())
        }
        Backend::Sqlite(b) => {
            let b = b.clone();
            let sql = format!("PRAGMA {}.table_xinfo({})", quote_sqlite(database), quote_sqlite(table));
            let table = table.to_string();
            tokio::task::spawn_blocking(move || {
                let conn = b.conn.lock().map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
                let rows: Vec<TableColumn> = stmt
                    .query_map([], |r| {
                        let ty: String = r.get(2)?;
                        let pk: i64 = r.get(5)?;
                        let hidden: i64 = r.get(6)?;
                        Ok(TableColumn {
                            name: r.get(1)?,
                            type_name: ty.to_lowercase(),
                            nullable: r.get::<_, i64>(3)? == 0,
                            default: r.get(4)?,
                            primary_key: pk > 0,
                            // INTEGER PRIMARY KEY is the rowid: SQLite assigns it.
                            auto_increment: pk == 1 && ty.eq_ignore_ascii_case("INTEGER"),
                            generated: hidden == 2 || hidden == 3,
                            binary: ty.to_ascii_uppercase().contains("BLOB"),
                        })
                    })
                    .map_err(|e| e.to_string())?
                    .filter_map(Result::ok)
                    .collect();
                if rows.is_empty() {
                    return Err(format!("Table {table} was not found."));
                }
                Ok(rows)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

// ---------- fuzzy search ----------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    #[serde(flatten)]
    pub object: DbObject,
    pub score: u16,
    /// Character positions in `name` that matched, ascending (for highlighting).
    pub indices: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortBy {
    Score,
    Name,
}

/// Fuzzy-match object names. Query syntax: `abc` fuzzy, `'abc` substring, `^abc` prefix,
/// `abc$` suffix, `^abc$` exact, `!abc` exclude; several words must all match.
pub fn search(objects: &[DbObject], query: &str, sort: SortBy, limit: usize) -> Vec<Hit> {
    let query = query.trim();
    if query.is_empty() {
        return objects
            .iter()
            .take(limit)
            .map(|o| Hit { object: o.clone(), score: 0, indices: vec![] })
            .collect();
    }
    let names: Vec<&str> = objects.iter().map(|o| o.name.as_str()).collect();
    let config = frizbee::Config { max_typos: Some(0), ..frizbee::Config::default() };
    let mut matcher = frizbee::Matcher::from_query(query, &config);
    let mut matches = matcher.match_list_indices(&names);
    match sort {
        SortBy::Score => matches.sort(),
        SortBy::Name => matches.sort_by_key(|m| m.index),
    }
    matches
        .into_iter()
        .take(limit)
        .map(|m| {
            let name = names[m.index as usize];
            Hit {
                object: objects[m.index as usize].clone(),
                score: m.score,
                indices: byte_to_char_indices(name, &m.indices),
            }
        })
        .collect()
}

fn byte_to_char_indices(s: &str, byte_indices: &[u32]) -> Vec<u32> {
    let mut out: Vec<u32> = byte_indices
        .iter()
        .filter_map(|&b| s.char_indices().position(|(i, _)| i == b as usize).map(|p| p as u32))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// Ctrl+P: search every database (or just `database`).
pub async fn search_session(
    session: &SessionHandle,
    query: &str,
    database: Option<&str>,
    sort: SortBy,
    limit: usize,
) -> Result<Vec<Hit>, String> {
    let list = match database {
        Some(db) => objects(session, db, false).await?,
        None => all_objects(session).await?,
    };
    Ok(search(&list, query, sort, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(name: &str) -> DbObject {
        DbObject { database: "d".into(), name: name.into(), kind: ObjectKind::Table, parent: None, rows: None, comment: None }
    }

    #[test]
    fn fuzzy_and_anchors() {
        let list: Vec<DbObject> = ["users", "user_roles", "orders", "order_items", "audit_users"].iter().map(|n| obj(n)).collect();
        let names = |hits: Vec<Hit>| hits.into_iter().map(|h| h.object.name).collect::<Vec<_>>();

        let fuzzy = names(search(&list, "usr", SortBy::Score, 10));
        assert!(fuzzy.contains(&"users".to_string()) && fuzzy.contains(&"user_roles".to_string()));
        assert!(!fuzzy.contains(&"orders".to_string()));

        let prefix = names(search(&list, "^order", SortBy::Name, 10));
        assert_eq!(prefix, vec!["orders", "order_items"]);

        let suffix = names(search(&list, "users$", SortBy::Name, 10));
        assert_eq!(suffix, vec!["users", "audit_users"]);

        let exact = names(search(&list, "^users$", SortBy::Name, 10));
        assert_eq!(exact, vec!["users"]);

        let hit = &search(&list, "oit", SortBy::Score, 1)[0];
        assert_eq!(hit.object.name, "order_items");
        assert_eq!(hit.indices.len(), 3);
    }
}
