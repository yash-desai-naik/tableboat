//! Saving grid edits. Every change is keyed by the row's primary key, values are bound as
//! parameters (never pasted into SQL), and everything runs in one transaction: if any
//! statement fails, or an UPDATE/DELETE doesn't hit exactly one row, nothing is saved.

use super::meta::{TableColumn, quote_mysql, quote_sqlite, table_columns};
use super::query::hex;
use super::{Backend, SessionHandle};
use mysql_async::prelude::*;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum EditValue {
    Text(String),
    Null,
    Default,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ColumnValue {
    pub column: String,
    pub value: EditValue,
}

#[derive(Debug, Clone, Deserialize)]
pub struct KeyPart {
    pub column: String,
    /// The primary-key value as shown in the grid (hex `0x…` for binary keys).
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub key: Vec<KeyPart>,
    pub set: Vec<ColumnValue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Insert {
    pub values: Vec<ColumnValue>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Delete {
    pub key: Vec<KeyPart>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Changes {
    #[serde(default)]
    pub deletes: Vec<Delete>,
    #[serde(default)]
    pub updates: Vec<Update>,
    #[serde(default)]
    pub inserts: Vec<Insert>,
}

/// A value to bind, or a literal keyword.
#[derive(Debug, Clone)]
enum Param {
    Text(String),
    Bytes(Vec<u8>),
    Null,
}

struct Stmt {
    /// What the statement is, for error messages ("Row 3 (update)").
    label: String,
    sql: String,
    params: Vec<Param>,
    /// UPDATE/DELETE must hit exactly one row.
    expect_one: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Dialect {
    Mysql,
    Sqlite,
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    let h = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"))?;
    if h.len() % 2 != 0 {
        return None;
    }
    (0..h.len()).step_by(2).map(|i| u8::from_str_radix(&h[i..i + 2], 16).ok()).collect()
}

fn param_for(col: Option<&TableColumn>, text: String) -> Result<Param, String> {
    match col {
        Some(c) if c.binary => match decode_hex(&text) {
            Some(bytes) => Ok(Param::Bytes(bytes)),
            None if text.is_empty() => Ok(Param::Bytes(vec![])),
            None => Err(format!(
                "\"{}\" is a binary column: enter its value as hex, e.g. 0x0A1B (got \"{}\").",
                c.name,
                if text.len() > 40 { format!("{}…", &text[..40]) } else { text }
            )),
        },
        _ => Ok(Param::Text(text)),
    }
}

fn build(dialect: Dialect, database: &str, table: &str, columns: &[TableColumn], changes: &Changes) -> Result<Vec<Stmt>, String> {
    let q = |s: &str| match dialect {
        Dialect::Mysql => quote_mysql(s),
        Dialect::Sqlite => quote_sqlite(s),
    };
    let target = format!("{}.{}", q(database), q(table));
    let col = |name: &str| columns.iter().find(|c| c.name == name);
    let pk: Vec<&TableColumn> = columns.iter().filter(|c| c.primary_key).collect();
    if pk.is_empty() && (!changes.updates.is_empty() || !changes.deletes.is_empty()) {
        return Err(format!("{table} has no primary key, so rows can't be changed or deleted safely."));
    }

    let where_clause = |key: &[KeyPart], params: &mut Vec<Param>| -> Result<String, String> {
        // The key must name exactly the primary-key columns.
        if key.len() != pk.len() || !pk.iter().all(|p| key.iter().any(|k| k.column == p.name)) {
            return Err("The row's primary key is incomplete; reload the table and try again.".into());
        }
        let mut parts = Vec::new();
        for k in key {
            match &k.value {
                None => parts.push(format!("{} IS NULL", q(&k.column))),
                Some(v) => {
                    parts.push(format!("{} = ?", q(&k.column)));
                    params.push(param_for(col(&k.column), v.clone())?);
                }
            }
        }
        Ok(parts.join(" AND "))
    };
    let limit = if dialect == Dialect::Mysql { " LIMIT 1" } else { "" };

    let mut out = Vec::new();
    for (i, d) in changes.deletes.iter().enumerate() {
        let mut params = Vec::new();
        let w = where_clause(&d.key, &mut params)?;
        out.push(Stmt { label: format!("deleted row {}", i + 1), sql: format!("DELETE FROM {target} WHERE {w}{limit}"), params, expect_one: true });
    }
    for (i, u) in changes.updates.iter().enumerate() {
        let mut params = Vec::new();
        let mut sets = Vec::new();
        for cv in &u.set {
            let c = col(&cv.column).ok_or_else(|| format!("{table} has no column \"{}\".", cv.column))?;
            if c.generated {
                return Err(format!("\"{}\" is a generated column and can't be edited.", c.name));
            }
            match &cv.value {
                EditValue::Text(t) => {
                    sets.push(format!("{} = ?", q(&c.name)));
                    params.push(param_for(Some(c), t.clone())?);
                }
                EditValue::Null => sets.push(format!("{} = NULL", q(&c.name))),
                EditValue::Default => {
                    if dialect == Dialect::Sqlite {
                        return Err(format!("SQLite can't set an existing value back to DEFAULT (\"{}\").", c.name));
                    }
                    sets.push(format!("{} = DEFAULT", q(&c.name)));
                }
            }
        }
        if sets.is_empty() {
            continue;
        }
        let w = where_clause(&u.key, &mut params)?;
        out.push(Stmt {
            label: format!("changed row {}", i + 1),
            sql: format!("UPDATE {target} SET {} WHERE {w}{limit}", sets.join(", ")),
            params,
            expect_one: true,
        });
    }
    for (i, ins) in changes.inserts.iter().enumerate() {
        let mut names = Vec::new();
        let mut values = Vec::new();
        let mut params = Vec::new();
        for cv in &ins.values {
            let c = col(&cv.column).ok_or_else(|| format!("{table} has no column \"{}\".", cv.column))?;
            // Generated columns are computed by the server; DEFAULT is the same as leaving it out.
            if c.generated || matches!(cv.value, EditValue::Default) {
                continue;
            }
            names.push(q(&c.name));
            match &cv.value {
                EditValue::Text(t) => {
                    values.push("?".to_string());
                    params.push(param_for(Some(c), t.clone())?);
                }
                EditValue::Null => values.push("NULL".into()),
                EditValue::Default => unreachable!(),
            }
        }
        let sql = if names.is_empty() {
            match dialect {
                Dialect::Mysql => format!("INSERT INTO {target} () VALUES ()"),
                Dialect::Sqlite => format!("INSERT INTO {target} DEFAULT VALUES"),
            }
        } else {
            format!("INSERT INTO {target} ({}) VALUES ({})", names.join(", "), values.join(", "))
        };
        out.push(Stmt { label: format!("new row {}", i + 1), sql, params, expect_one: false });
    }
    Ok(out)
}

/// The SQL Save would run, with values written inline, for the Preview dialog.
pub async fn preview(session: &SessionHandle, database: &str, table: &str, changes: &Changes) -> Result<Vec<String>, String> {
    if let Backend::Mongo(b) = &session.backend {
        return super::mongo::preview(b, database, table, changes).await;
    }
    let columns = table_columns(session, database, table).await?;
    let dialect = dialect_of(session);
    let stmts = build(dialect, database, table, &columns, changes)?;
    Ok(stmts
        .into_iter()
        .map(|s| {
            let mut sql = String::new();
            let mut params = s.params.into_iter();
            for ch in s.sql.chars() {
                if ch == '?' {
                    sql.push_str(&match params.next() {
                        Some(Param::Text(t)) => match dialect {
                            Dialect::Mysql => format!("'{}'", t.replace('\\', "\\\\").replace('\'', "''")),
                            Dialect::Sqlite => format!("'{}'", t.replace('\'', "''")),
                        },
                        Some(Param::Bytes(b)) => match dialect {
                            Dialect::Mysql => hex(&b),
                            Dialect::Sqlite => format!("X'{}'", &hex(&b)[2..]),
                        },
                        Some(Param::Null) | None => "NULL".into(),
                    });
                } else {
                    sql.push(ch);
                }
            }
            format!("{sql};")
        })
        .collect())
}

fn dialect_of(session: &SessionHandle) -> Dialect {
    match session.backend {
        Backend::Mysql(_) => Dialect::Mysql,
        Backend::Sqlite(_) | Backend::Mongo(_) => Dialect::Sqlite,
    }
}

/// Apply all changes in one transaction. Returns the number of statements run.
pub async fn save(session: &SessionHandle, database: &str, table: &str, changes: &Changes) -> Result<usize, String> {
    if let Backend::Mongo(b) = &session.backend {
        return super::mongo::save(b, database, table, changes).await;
    }
    let columns = table_columns(session, database, table).await?;
    let dialect = dialect_of(session);
    let stmts = build(dialect, database, table, &columns, changes)?;
    if stmts.is_empty() {
        return Ok(0);
    }
    let count = stmts.len();
    match &session.backend {
        Backend::Mysql(b) => {
            let mut conn = b.pool.get_conn().await.map_err(|e| super::query::mysql_message(&e))?;
            let mut tx = conn
                .start_transaction(mysql_async::TxOpts::default())
                .await
                .map_err(|e| super::query::mysql_message(&e))?;
            for s in stmts {
                let params: Vec<mysql_async::Value> = s
                    .params
                    .into_iter()
                    .map(|p| match p {
                        Param::Text(t) => mysql_async::Value::Bytes(t.into_bytes()),
                        Param::Bytes(b) => mysql_async::Value::Bytes(b),
                        Param::Null => mysql_async::Value::NULL,
                    })
                    .collect();
                let result = tx.exec_drop(&s.sql, mysql_async::Params::Positional(params)).await;
                let problem = match result {
                    Err(e) => Some(super::query::mysql_message(&e)),
                    // client_found_rows: "affected" counts matched rows, so an unchanged value still counts.
                    Ok(()) if s.expect_one && tx.affected_rows() != 1 => Some(format!(
                        "it matched {} rows instead of 1 – the row may have been changed or deleted since it was loaded",
                        tx.affected_rows()
                    )),
                    Ok(()) => None,
                };
                if let Some(msg) = problem {
                    let _ = tx.rollback().await;
                    return Err(format!("Nothing was saved. The {} failed: {msg}", s.label));
                }
            }
            tx.commit().await.map_err(|e| format!("Nothing was saved: {}", super::query::mysql_message(&e)))?;
            Ok(count)
        }
        Backend::Mongo(_) => unreachable!("handled above"),
        Backend::Sqlite(b) => {
            if b.read_only {
                return Err("This SQLite file was opened read-only (it's on a network or WSL path).".into());
            }
            let b = b.clone();
            tokio::task::spawn_blocking(move || {
                let mut conn = b.conn.lock().map_err(|e| e.to_string())?;
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                for s in stmts {
                    let params: Vec<rusqlite::types::Value> = s
                        .params
                        .into_iter()
                        .map(|p| match p {
                            Param::Text(t) => rusqlite::types::Value::Text(t),
                            Param::Bytes(b) => rusqlite::types::Value::Blob(b),
                            Param::Null => rusqlite::types::Value::Null,
                        })
                        .collect();
                    let problem = match tx.execute(&s.sql, rusqlite::params_from_iter(params)) {
                        Err(e) => Some(e.to_string()),
                        Ok(n) if s.expect_one && n != 1 => Some(format!(
                            "it matched {n} rows instead of 1 – the row may have been changed or deleted since it was loaded"
                        )),
                        Ok(_) => None,
                    };
                    if let Some(msg) = problem {
                        // Dropping the transaction rolls it back.
                        return Err(format!("Nothing was saved. The {} failed: {msg}", s.label));
                    }
                }
                tx.commit().map_err(|e| format!("Nothing was saved: {e}"))?;
                Ok(count)
            })
            .await
            .map_err(|e| e.to_string())?
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols() -> Vec<TableColumn> {
        let c = |name: &str, pk: bool, generated: bool, binary: bool| TableColumn {
            name: name.into(),
            type_name: "x".into(),
            nullable: true,
            default: None,
            primary_key: pk,
            auto_increment: pk,
            generated,
            binary,
        };
        vec![c("id", true, false, false), c("name", false, false, false), c("full", false, true, false), c("uuid", false, false, true)]
    }

    #[test]
    fn builds_parameterised_statements() {
        let changes: Changes = serde_json::from_str(
            r#"{
              "deletes": [{"key": [{"column": "id", "value": "7"}]}],
              "updates": [{"key": [{"column": "id", "value": "3"}],
                           "set": [{"column": "name", "value": {"kind": "text", "value": "888--"}},
                                   {"column": "uuid", "value": {"kind": "null"}}]}],
              "inserts": [{"values": [{"column": "id", "value": {"kind": "default"}},
                                      {"column": "name", "value": {"kind": "text", "value": "O'Brien"}},
                                      {"column": "full", "value": {"kind": "text", "value": "ignored"}},
                                      {"column": "uuid", "value": {"kind": "text", "value": "0x00FF"}}]}]
            }"#,
        )
        .unwrap();
        let s = build(Dialect::Mysql, "shop", "people", &cols(), &changes).unwrap();
        assert_eq!(s[0].sql, "DELETE FROM `shop`.`people` WHERE `id` = ? LIMIT 1");
        assert_eq!(s[1].sql, "UPDATE `shop`.`people` SET `name` = ?, `uuid` = NULL WHERE `id` = ? LIMIT 1");
        assert!(matches!(&s[1].params[0], Param::Text(t) if t == "888--"), "values are bound, never pasted");
        assert_eq!(s[2].sql, "INSERT INTO `shop`.`people` (`name`, `uuid`) VALUES (?, ?)", "DEFAULT and generated columns left out");
        assert!(matches!(&s[2].params[1], Param::Bytes(b) if b == &vec![0u8, 255]));

        let none = build(Dialect::Sqlite, "main", "t", &cols(), &serde_json::from_str(r#"{"inserts":[{"values":[]}]}"#).unwrap()).unwrap();
        assert_eq!(none[0].sql, "INSERT INTO \"main\".\"t\" DEFAULT VALUES");
    }

    #[test]
    fn refuses_unsafe_changes() {
        let no_pk: Vec<TableColumn> = cols().into_iter().map(|mut c| { c.primary_key = false; c }).collect();
        let del: Changes = serde_json::from_str(r#"{"deletes":[{"key":[{"column":"id","value":"1"}]}]}"#).unwrap();
        assert!(build(Dialect::Mysql, "d", "t", &no_pk, &del).is_err(), "no primary key: no deletes");
        let partial: Changes = serde_json::from_str(r#"{"deletes":[{"key":[]}]}"#).unwrap();
        assert!(build(Dialect::Mysql, "d", "t", &cols(), &partial).is_err(), "empty key never becomes a bare DELETE");
        let generated: Changes = serde_json::from_str(r#"{"updates":[{"key":[{"column":"id","value":"1"}],"set":[{"column":"full","value":{"kind":"null"}}]}]}"#).unwrap();
        assert!(build(Dialect::Mysql, "d", "t", &cols(), &generated).is_err(), "generated column");
    }
}
