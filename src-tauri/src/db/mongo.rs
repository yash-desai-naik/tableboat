//! MongoDB through the official Rust driver (rustls, no OpenSSL). Documents become grid rows:
//! top-level fields are columns (`_id` first), nested objects/arrays show as compact JSON.

use super::ServerInfo;
use super::edit::{Changes, EditValue};
use super::meta::{DbObject, ObjectKind, TableColumn};
use super::mongo_shell::{self, Command, Op};
use super::query::{StatementOutcome, hex};
use super::results::{Cell, Column, ResultSet};
use crate::conn::model::{Connection, Engine, Secrets, SslMode};
use futures_util::TryStreamExt;
use mongodb::bson::{self, Bson, Document, doc, spec::ElementType};
use mongodb::options::ClientOptions;
use mongodb::{Client, Database};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

#[derive(Clone)]
pub struct MongoBackend {
    pub client: Client,
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// A connection string from the saved settings. `tunnel` = local port of an SSH tunnel.
fn build_uri(conn: &Connection, secrets: &Secrets, tunnel: Option<u16>) -> String {
    let srv = tunnel.is_none() && conn.options.get("srv").map(|v| v == "true").unwrap_or(false);
    let mut uri = String::from(if srv { "mongodb+srv://" } else { "mongodb://" });
    if !conn.user.is_empty() {
        uri.push_str(&enc(&conn.user));
        if let Some(p) = secrets.password.as_deref().filter(|p| !p.is_empty()) {
            uri.push(':');
            uri.push_str(&enc(p));
        }
        uri.push('@');
    }
    let host = conn.host.trim();
    match tunnel {
        Some(port) => uri.push_str(&format!("127.0.0.1:{port}")),
        None if srv || host.contains(',') => uri.push_str(host),
        None => uri.push_str(&format!("{host}:{}", conn.effective_port())),
    }
    uri.push('/');

    let mut params: Vec<(String, String)> = conn
        .options
        .iter()
        .filter(|(k, _)| k.as_str() != "srv")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let has = |params: &Vec<(String, String)>, k: &str| params.iter().any(|(p, _)| p.eq_ignore_ascii_case(k));
    let tls = match conn.ssl.mode {
        SslMode::Disabled => Some(false),
        SslMode::Preferred => None,
        SslMode::Required | SslMode::VerifyCa | SslMode::VerifyIdentity => Some(true),
    };
    if let Some(t) = tls {
        if !has(&params, "tls") && !has(&params, "ssl") {
            params.push(("tls".into(), t.to_string()));
        }
    }
    if conn.ssl.mode == SslMode::Required {
        params.push(("tlsAllowInvalidCertificates".into(), "true".into()));
    }
    if conn.ssl.mode == SslMode::VerifyCa || (tunnel.is_some() && tls == Some(true)) {
        params.push(("tlsAllowInvalidHostnames".into(), "true".into()));
    }
    if let Some(ca) = conn.ssl.ca_file.as_deref().filter(|s| !s.is_empty()) {
        params.push(("tlsCAFile".into(), ca.into()));
    }
    // MongoDB takes the client certificate and key as one PEM file.
    if let Some(cert) = conn.ssl.cert_file.as_deref().filter(|s| !s.is_empty()) {
        params.push(("tlsCertificateKeyFile".into(), cert.into()));
    }
    if tunnel.is_some() && !has(&params, "directConnection") {
        params.push(("directConnection".into(), "true".into()));
    }
    for (k, v) in [("appName", "Tableboat"), ("serverSelectionTimeoutMS", "10000"), ("connectTimeoutMS", "10000"), ("maxPoolSize", "4")] {
        if !has(&params, k) {
            params.push((k.into(), v.into()));
        }
    }
    uri.push('?');
    uri.push_str(&params.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&"));
    uri
}

fn describe(e: &mongodb::error::Error, conn: &Connection) -> String {
    let text = e.to_string();
    let lower = text.to_lowercase();
    if lower.contains("authentication failed") || lower.contains("auth error") {
        format!(
            "Authentication failed for user \"{}\". Check the user, password and auth database{}.",
            conn.user,
            conn.options.get("authSource").map(|a| format!(" (now \"{a}\")")).unwrap_or_else(|| " (default: admin)".into())
        )
    } else if lower.contains("server selection timeout") {
        format!("Couldn't reach MongoDB at {} within 10 s. Is it running, and are the host, port and TLS settings right?\n\nDetails: {text}", conn.host)
    } else {
        text
    }
}

pub async fn connect(conn: &Connection, secrets: &Secrets, tunnel: Option<u16>) -> Result<(MongoBackend, ServerInfo), String> {
    if conn.host.trim().is_empty() {
        return Err("Enter the host.".into());
    }
    let uri = build_uri(conn, secrets, tunnel);
    let options = ClientOptions::parse(&uri).await.map_err(|e| describe(&e, conn))?;
    let tls = options.tls.as_ref().map(|t| matches!(t, mongodb::options::Tls::Enabled(_))).unwrap_or(false);
    let client = Client::with_options(options).map_err(|e| describe(&e, conn))?;
    let admin = client.database("admin");
    admin.run_command(doc! { "ping": 1 }).await.map_err(|e| describe(&e, conn))?;
    let version = admin
        .run_command(doc! { "buildInfo": 1 })
        .await
        .ok()
        .and_then(|d| d.get_str("version").ok().map(str::to_string))
        .unwrap_or_default();
    let info = ServerInfo {
        engine: Some(Engine::Mongodb),
        product: "MongoDB".into(),
        version: version.clone(),
        full_version: version,
        database: Some(conn.database.clone()).filter(|d| !d.is_empty()),
        tls,
        ssh: false,
    };
    Ok((MongoBackend { client }, info))
}

fn err(e: mongodb::error::Error) -> String {
    e.to_string()
}

// ---------- metadata ----------

pub async fn databases(b: &MongoBackend, fallback: Option<&str>) -> Result<Vec<String>, String> {
    match b.client.list_database_names().await {
        Ok(mut names) => {
            let system = ["admin", "config", "local"];
            names.sort_by_key(|n| (system.contains(&n.as_str()), n.to_lowercase()));
            Ok(names)
        }
        // A user without listDatabases can still use their own database.
        Err(e) => match fallback {
            Some(db) => Ok(vec![db.to_string()]),
            None => Err(format!("{} (set a Database in the connection to open it directly)", err(e))),
        },
    }
}

pub async fn objects(b: &MongoBackend, database: &str) -> Result<Vec<DbObject>, String> {
    let mut cursor = b.client.database(database).list_collections().await.map_err(err)?;
    let mut out = Vec::new();
    while let Some(spec) = cursor.try_next().await.map_err(err)? {
        if spec.name.starts_with("system.") {
            continue;
        }
        let view = matches!(spec.collection_type, mongodb::results::CollectionType::View);
        out.push(DbObject {
            database: database.to_string(),
            name: spec.name,
            kind: if view { ObjectKind::View } else { ObjectKind::Table },
            parent: None,
            rows: None,
            comment: None,
        });
    }
    out.sort_by(|a, c| a.name.to_lowercase().cmp(&c.name.to_lowercase()));
    Ok(out)
}

/// Collection options and indexes, as readable JSON.
pub async fn definition(b: &MongoBackend, database: &str, name: &str) -> Result<String, String> {
    let db = b.client.database(database);
    let listed = db.run_command(doc! { "listCollections": 1, "filter": { "name": name } }).await.map_err(err)?;
    let info = listed
        .get_document("cursor")
        .ok()
        .and_then(|c| c.get_array("firstBatch").ok())
        .and_then(|a| a.first().cloned())
        .unwrap_or(Bson::Null);
    let indexes = db
        .run_command(doc! { "listIndexes": name })
        .await
        .ok()
        .and_then(|d| d.get_document("cursor").ok().and_then(|c| c.get_array("firstBatch").ok().cloned()))
        .unwrap_or_default();
    let pretty = |v: Bson| serde_json::to_string_pretty(&v.into_relaxed_extjson()).unwrap_or_default();
    Ok(format!(
        "// Collection {database}.{name}\n{}\n\n// Indexes\n{}\n",
        pretty(info),
        pretty(Bson::Array(indexes))
    ))
}

fn type_name(b: &Bson) -> &'static str {
    match b.element_type() {
        ElementType::ObjectId => "objectId",
        ElementType::String => "string",
        ElementType::Int32 => "int",
        ElementType::Int64 => "long",
        ElementType::Double => "double",
        ElementType::Decimal128 => "decimal",
        ElementType::Boolean => "bool",
        ElementType::DateTime => "date",
        ElementType::EmbeddedDocument => "object",
        ElementType::Array => "array",
        ElementType::Binary => "binData",
        ElementType::Null | ElementType::Undefined => "null",
        ElementType::RegularExpression => "regex",
        ElementType::Timestamp => "timestamp",
        _ => "other",
    }
}

fn is_numeric(b: &Bson) -> bool {
    matches!(b, Bson::Int32(_) | Bson::Int64(_) | Bson::Double(_) | Bson::Decimal128(_))
}

/// Field names and their most common type, from a sample of documents (for editing and Claude).
pub async fn sample_fields(b: &MongoBackend, database: &str, collection: &str, sample: i64) -> Result<Vec<TableColumn>, String> {
    let coll = b.client.database(database).collection::<Document>(collection);
    let mut cursor = coll.find(doc! {}).limit(sample).await.map_err(err)?;
    let mut order: Vec<String> = vec!["_id".into()];
    let mut types: HashMap<String, HashMap<&'static str, usize>> = HashMap::new();
    while let Some(d) = cursor.try_next().await.map_err(err)? {
        for (k, v) in d.iter() {
            if !order.contains(k) {
                order.push(k.clone());
            }
            if !matches!(v, Bson::Null) {
                *types.entry(k.clone()).or_default().entry(type_name(v)).or_default() += 1;
            }
        }
    }
    Ok(order
        .into_iter()
        .map(|name| {
            let ty = types
                .get(&name)
                .and_then(|t| t.iter().max_by_key(|(_, n)| **n).map(|(t, _)| *t))
                .unwrap_or(if name == "_id" { "objectId" } else { "string" });
            TableColumn {
                primary_key: name == "_id",
                // MongoDB fills in a new _id when it's left out.
                auto_increment: name == "_id",
                nullable: name != "_id",
                default: None,
                generated: false,
                binary: false,
                type_name: ty.to_string(),
                name,
            }
        })
        .collect())
}

// ---------- documents → rows ----------

/// How a value shows in the grid (and how it is copied).
pub fn display(b: &Bson) -> Cell {
    let s = match b {
        Bson::Null | Bson::Undefined => return None,
        Bson::String(s) => s.clone(),
        Bson::Int32(n) => n.to_string(),
        Bson::Int64(n) => n.to_string(),
        Bson::Double(f) => {
            if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 { format!("{f:.1}") } else { f.to_string() }
        }
        Bson::Boolean(v) => v.to_string(),
        Bson::ObjectId(o) => format!("ObjectId('{o}')"),
        Bson::DateTime(d) => d.try_to_rfc3339_string().unwrap_or_else(|_| d.timestamp_millis().to_string()),
        Bson::Binary(bin) if bin.subtype == bson::spec::BinarySubtype::Uuid && bin.bytes.len() == 16 => {
            format!("UUID('{}')", uuid::Uuid::from_slice(&bin.bytes).map(|u| u.to_string()).unwrap_or_default())
        }
        Bson::Binary(bin) => hex(&bin.bytes),
        Bson::Decimal128(_) => match b.clone().into_relaxed_extjson() {
            serde_json::Value::Object(m) => m.get("$numberDecimal").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            other => other.to_string(),
        },
        other => serde_json::to_string(&other.clone().into_relaxed_extjson()).unwrap_or_default(),
    };
    Some(s.into_boxed_str())
}

pub fn documents_to_result(docs: Vec<Document>, truncated: bool) -> ResultSet {
    let mut order: Vec<String> = Vec::new();
    for d in &docs {
        for k in d.keys() {
            if !order.contains(k) && order.len() < 1000 {
                if k == "_id" { order.insert(0, k.clone()) } else { order.push(k.clone()) }
            }
        }
    }
    let mut types: Vec<HashMap<&'static str, usize>> = vec![HashMap::new(); order.len()];
    let mut numeric = vec![true; order.len()];
    let mut seen = vec![false; order.len()];
    let rows: Vec<Box<[Cell]>> = docs
        .iter()
        .map(|d| {
            order
                .iter()
                .enumerate()
                .map(|(i, k)| {
                    let v = d.get(k);
                    if let Some(v) = v.filter(|v| !matches!(v, Bson::Null)) {
                        seen[i] = true;
                        numeric[i] &= is_numeric(v);
                        *types[i].entry(type_name(v)).or_default() += 1;
                    }
                    v.and_then(display)
                })
                .collect()
        })
        .collect();
    let columns = order
        .iter()
        .enumerate()
        .map(|(i, name)| Column {
            name: name.clone(),
            type_name: types[i].iter().max_by_key(|(_, n)| **n).map(|(t, _)| t.to_string()).unwrap_or_default(),
            table: None,
            numeric: seen[i] && numeric[i],
            binary: false,
            primary_key: name == "_id",
        })
        .collect();
    ResultSet::new(columns, rows, truncated)
}

fn one_row(fields: Vec<(&str, Bson)>) -> ResultSet {
    let mut d = Document::new();
    for (k, v) in fields {
        d.insert(k, v);
    }
    documents_to_result(vec![d], false)
}

// ---------- running shell statements ----------

pub async fn run(
    b: &MongoBackend,
    statements: Vec<String>,
    database: Option<String>,
    max_rows: usize,
    stop_on_error: bool,
    cancelled: Arc<AtomicBool>,
    cancel: Arc<Notify>,
) -> Vec<StatementOutcome> {
    let mut current_db = database.filter(|d| !d.is_empty());
    let mut out = Vec::new();
    for (index, sql) in statements.iter().enumerate() {
        let mut o = StatementOutcome::new(index, sql);
        if cancelled.load(Ordering::SeqCst) {
            o.cancelled = true;
            o.error = Some("Cancelled before it ran.".into());
            out.push(o);
            continue;
        }
        let started = Instant::now();
        let result = tokio::select! {
            r = run_one(b, sql, &mut current_db, max_rows, &mut o) => r,
            _ = cancel.notified() => Err("Cancelled.".to_string()),
        };
        o.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        if let Err(e) = result {
            o.cancelled = cancelled.load(Ordering::SeqCst);
            o.error = Some(e);
            out.push(o);
            if stop_on_error || cancelled.load(Ordering::SeqCst) {
                break;
            }
            continue;
        }
        out.push(o);
    }
    out
}

async fn update(
    coll: &mongodb::Collection<Document>,
    filter: Document,
    update: Bson,
    upsert: bool,
    many: bool,
    o: &mut StatementOutcome,
) -> Result<(), String> {
    let modifications: mongodb::options::UpdateModifications = match update {
        Bson::Document(d) => d.into(),
        Bson::Array(stages) => stages.into_iter().filter_map(|s| s.as_document().cloned()).collect::<Vec<_>>().into(),
        _ => return Err("The update must be an object or a pipeline array.".into()),
    };
    let r = if many {
        coll.update_many(filter, modifications).upsert(upsert).await
    } else {
        coll.update_one(filter, modifications).upsert(upsert).await
    }
    .map_err(err)?;
    o.affected_rows = Some(r.modified_count);
    o.info = Some(format!(
        "{} matched, {} modified{}",
        r.matched_count,
        r.modified_count,
        r.upserted_id.as_ref().and_then(display).map(|id| format!(", upserted _id {id}")).unwrap_or_default()
    ));
    Ok(())
}

async fn collect(mut cursor: mongodb::Cursor<Document>, max_rows: usize) -> Result<(Vec<Document>, bool), String> {
    let mut docs = Vec::new();
    while let Some(d) = cursor.try_next().await.map_err(err)? {
        if docs.len() >= max_rows {
            // Dropping the cursor kills it on the server: the rest isn't downloaded.
            return Ok((docs, true));
        }
        docs.push(d);
    }
    Ok((docs, false))
}

async fn run_one(b: &MongoBackend, sql: &str, current_db: &mut Option<String>, max_rows: usize, o: &mut StatementOutcome) -> Result<(), String> {
    let command = mongo_shell::parse(sql)?;
    let db = |current: &Option<String>| -> Result<Database, String> {
        current
            .as_ref()
            .map(|name| b.client.database(name))
            .ok_or_else(|| "Choose a database first (Ctrl+K), or start with: use <database>".to_string())
    };
    match command {
        Command::ShowDatabases => {
            let names = databases(b, current_db.as_deref()).await?;
            let docs = names.into_iter().map(|n| doc! { "name": n }).collect();
            o.result = Some(documents_to_result(docs, false));
        }
        Command::ShowCollections | Command::CollectionNames => {
            let names = objects(b, current_db.as_deref().ok_or("Choose a database first (Ctrl+K)")?).await?;
            let docs = names
                .into_iter()
                .map(|x| doc! { "name": x.name, "type": if x.kind == ObjectKind::View { "view" } else { "collection" } })
                .collect();
            o.result = Some(documents_to_result(docs, false));
        }
        Command::Use(name) => {
            *current_db = Some(name.clone());
            o.info = Some(format!("Switched to {name} for the rest of this run."));
        }
        Command::RunCommand { admin, command } => {
            let target = if admin { b.client.database("admin") } else { db(current_db)? };
            let reply = target.run_command(command).await.map_err(err)?;
            o.result = Some(documents_to_result(vec![reply], false));
        }
        Command::Collection { name, op } => {
            let coll = db(current_db)?.collection::<Document>(&name);
            match op {
                Op::Find { filter, projection, sort, skip, limit, one: _, count } => {
                    if count {
                        let n = coll.count_documents(filter).await.map_err(err)?;
                        o.result = Some(one_row(vec![("count", Bson::Int64(n as i64))]));
                        return Ok(());
                    }
                    let mut find = coll.find(filter);
                    if let Some(p) = projection {
                        find = find.projection(p);
                    }
                    if let Some(s) = sort {
                        find = find.sort(s);
                    }
                    if let Some(s) = skip {
                        find = find.skip(s);
                    }
                    if let Some(l) = limit {
                        find = find.limit(l);
                    }
                    let (docs, truncated) = collect(find.await.map_err(err)?, max_rows).await?;
                    o.result = Some(documents_to_result(docs, truncated));
                }
                Op::Aggregate { pipeline } => {
                    let (docs, truncated) = collect(coll.aggregate(pipeline).await.map_err(err)?, max_rows).await?;
                    o.result = Some(documents_to_result(docs, truncated));
                }
                Op::CountDocuments { filter } => {
                    let n = coll.count_documents(filter).await.map_err(err)?;
                    o.result = Some(one_row(vec![("count", Bson::Int64(n as i64))]));
                }
                Op::EstimatedCount => {
                    let n = coll.estimated_document_count().await.map_err(err)?;
                    o.result = Some(one_row(vec![("count", Bson::Int64(n as i64))]));
                }
                Op::Distinct { field, filter } => {
                    let values = coll.distinct(&field, filter).await.map_err(err)?;
                    let docs = values.into_iter().map(|v| doc! { field.clone(): v }).collect();
                    o.result = Some(documents_to_result(docs, false));
                }
                Op::InsertOne { doc } => {
                    let r = coll.insert_one(doc).await.map_err(err)?;
                    o.affected_rows = Some(1);
                    o.info = display(&r.inserted_id).map(|id| format!("Inserted _id {id}"));
                }
                Op::InsertMany { docs } => {
                    let r = coll.insert_many(docs).await.map_err(err)?;
                    o.affected_rows = Some(r.inserted_ids.len() as u64);
                }
                Op::UpdateOne { filter, update: u, upsert } => update(&coll, filter, u, upsert, false, o).await?,
                Op::UpdateMany { filter, update: u, upsert } => update(&coll, filter, u, upsert, true, o).await?,
                Op::ReplaceOne { filter, doc, upsert } => {
                    let r = coll.replace_one(filter, doc).upsert(upsert).await.map_err(err)?;
                    o.affected_rows = Some(r.modified_count);
                    o.info = Some(format!("{} matched, {} modified", r.matched_count, r.modified_count));
                }
                Op::DeleteOne { filter } => {
                    let r = coll.delete_one(filter).await.map_err(err)?;
                    o.affected_rows = Some(r.deleted_count);
                }
                Op::DeleteMany { filter } => {
                    let r = coll.delete_many(filter).await.map_err(err)?;
                    o.affected_rows = Some(r.deleted_count);
                }
                Op::CreateIndex { keys, options } => {
                    let mut model = mongodb::IndexModel::builder().keys(keys).build();
                    if let Some(opts) = options {
                        model.options = Some(bson::from_document(opts).map_err(|e| format!("Index options: {e}"))?);
                    }
                    let r = coll.create_index(model).await.map_err(err)?;
                    o.info = Some(format!("Created index {}", r.index_name));
                }
                Op::DropIndex { name } => {
                    coll.drop_index(&name).await.map_err(err)?;
                    o.info = Some(format!("Dropped index {name}"));
                }
                Op::GetIndexes => {
                    let reply = db(current_db)?.run_command(doc! { "listIndexes": &name }).await.map_err(err)?;
                    let docs = reply
                        .get_document("cursor")
                        .ok()
                        .and_then(|c| c.get_array("firstBatch").ok())
                        .map(|a| a.iter().filter_map(|b| b.as_document().cloned()).collect())
                        .unwrap_or_default();
                    o.result = Some(documents_to_result(docs, false));
                }
                Op::Drop => {
                    coll.drop().await.map_err(err)?;
                    o.info = Some(format!("Dropped collection {name}"));
                }
            }
        }
    }
    Ok(())
}

// ---------- saving grid edits ----------

/// Turn typed text into a value of the field's type (number stays number, date stays date…).
fn convert(text: &str, type_name: &str, field: &str) -> Result<Bson, String> {
    let t = text.trim();
    let bad = |what: &str| format!("\"{field}\" holds {what}; \"{}\" isn't one.", if t.len() > 40 { &t[..40] } else { t });
    match type_name {
        "objectId" => {
            if t.starts_with("ObjectId") {
                mongo_shell::parse_value(t)
            } else {
                bson::oid::ObjectId::parse_str(t).map(Bson::ObjectId).map_err(|_| bad("an ObjectId"))
            }
        }
        "int" => t.parse::<i32>().map(Bson::Int32).or_else(|_| t.parse::<i64>().map(Bson::Int64)).map_err(|_| bad("a whole number")),
        "long" => t.parse::<i64>().map(Bson::Int64).map_err(|_| bad("a whole number")),
        "double" => t.parse::<f64>().map(Bson::Double).map_err(|_| bad("a number")),
        "decimal" => mongo_shell::parse_value(&format!("NumberDecimal('{t}')")).map_err(|_| bad("a decimal number")),
        "bool" => match t.to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(Bson::Boolean(true)),
            "false" | "0" => Ok(Bson::Boolean(false)),
            _ => Err(bad("true or false")),
        },
        "date" => mongo_shell::parse_date(t).map(Bson::DateTime).map_err(|e| format!("\"{field}\": {e}")),
        "object" | "array" => {
            let v = mongo_shell::parse_value(t).map_err(|e| format!("\"{field}\" holds {type_name}s: {e}"))?;
            match (&v, type_name) {
                (Bson::Document(_), "object") | (Bson::Array(_), "array") => Ok(v),
                _ => Err(bad(if type_name == "object" { "an object { … }" } else { "an array [ … ]" })),
            }
        }
        "binData" if t.starts_with("UUID") => mongo_shell::parse_value(t),
        _ => Ok(Bson::String(text.to_string())),
    }
}

/// The _id as shown in the grid → candidate values (a string like "123" may be a number _id).
fn key_candidates(shown: &str) -> Vec<Bson> {
    let t = shown.trim();
    if t.starts_with("ObjectId(") || t.starts_with("UUID(") {
        return mongo_shell::parse_value(t).into_iter().collect();
    }
    let mut out = vec![Bson::String(shown.to_string())];
    if let Ok(n) = t.parse::<i64>() {
        out.push(i32::try_from(n).map(Bson::Int32).unwrap_or(Bson::Int64(n)));
        out.push(Bson::Int64(n));
        out.push(Bson::Double(n as f64));
    } else if let Ok(f) = t.parse::<f64>() {
        out.push(Bson::Double(f));
    }
    out
}

async fn find_by_key(coll: &mongodb::Collection<Document>, shown: &Option<String>) -> Result<(Bson, Document), String> {
    let shown = shown.as_deref().ok_or("A document without _id can't be changed.")?;
    for candidate in key_candidates(shown) {
        if let Some(doc) = coll.find_one(doc! { "_id": candidate.clone() }).await.map_err(err)? {
            return Ok((candidate, doc));
        }
    }
    Err(format!("The document with _id {shown} no longer exists (changed or deleted since it was loaded)."))
}

fn shell(v: &Bson) -> String {
    match v {
        Bson::ObjectId(o) => format!("ObjectId('{o}')"),
        Bson::DateTime(d) => format!("ISODate('{}')", d.try_to_rfc3339_string().unwrap_or_default()),
        other => serde_json::to_string(&other.clone().into_relaxed_extjson()).unwrap_or_default(),
    }
}

struct Planned {
    label: String,
    preview: String,
    action: Action,
}

enum Action {
    Delete(Bson),
    Update(Bson, Document),
    Insert(Document),
}

async fn plan(b: &MongoBackend, database: &str, collection: &str, changes: &Changes) -> Result<Vec<Planned>, String> {
    let coll = b.client.database(database).collection::<Document>(collection);
    let fields = sample_fields(b, database, collection, 100).await?;
    let field_type = |name: &str| fields.iter().find(|f| f.name == name).map(|f| f.type_name.clone()).unwrap_or_else(|| "string".into());
    let target = format!("db.getCollection({})", serde_json::to_string(collection).unwrap());
    let key_of = |key: &[super::edit::KeyPart]| key.iter().find(|k| k.column == "_id").map(|k| k.value.clone());
    let mut out = Vec::new();

    for (i, d) in changes.deletes.iter().enumerate() {
        let (id, _) = find_by_key(&coll, &key_of(&d.key).ok_or("Missing _id")?).await?;
        out.push(Planned {
            label: format!("deleted document {}", i + 1),
            preview: format!("{target}.deleteOne({{ _id: {} }})", shell(&id)),
            action: Action::Delete(id),
        });
    }
    for (i, u) in changes.updates.iter().enumerate() {
        let (id, original) = find_by_key(&coll, &key_of(&u.key).ok_or("Missing _id")?).await?;
        let mut set = Document::new();
        let mut unset = Document::new();
        for cv in &u.set {
            if cv.column == "_id" {
                return Err("_id can't be changed; duplicate the document instead.".into());
            }
            match &cv.value {
                EditValue::Null => {
                    set.insert(cv.column.clone(), Bson::Null);
                }
                EditValue::Default => {
                    unset.insert(cv.column.clone(), "");
                }
                EditValue::Text(t) => {
                    // Keep the field's current type; for a new field, the collection's usual type.
                    let ty = match original.get(&cv.column) {
                        Some(v) if !matches!(v, Bson::Null) => type_name(v).to_string(),
                        _ => field_type(&cv.column),
                    };
                    set.insert(cv.column.clone(), convert(t, &ty, &cv.column)?);
                }
            }
        }
        let mut update = Document::new();
        if !set.is_empty() {
            update.insert("$set", set);
        }
        if !unset.is_empty() {
            update.insert("$unset", unset);
        }
        if update.is_empty() {
            continue;
        }
        out.push(Planned {
            label: format!("changed document {}", i + 1),
            preview: format!("{target}.updateOne({{ _id: {} }}, {})", shell(&id), shell(&Bson::Document(update.clone()))),
            action: Action::Update(id, update),
        });
    }
    for (i, ins) in changes.inserts.iter().enumerate() {
        let mut d = Document::new();
        for cv in &ins.values {
            match &cv.value {
                EditValue::Default => {}
                EditValue::Null => {
                    d.insert(cv.column.clone(), Bson::Null);
                }
                EditValue::Text(t) => {
                    d.insert(cv.column.clone(), convert(t, &field_type(&cv.column), &cv.column)?);
                }
            }
        }
        out.push(Planned {
            label: format!("new document {}", i + 1),
            preview: format!("{target}.insertOne({})", shell(&Bson::Document(d.clone()))),
            action: Action::Insert(d),
        });
    }
    Ok(out)
}

pub async fn preview(b: &MongoBackend, database: &str, collection: &str, changes: &Changes) -> Result<Vec<String>, String> {
    Ok(plan(b, database, collection, changes).await?.into_iter().map(|p| format!("{};", p.preview)).collect())
}

/// MongoDB without a replica set has no transactions: changes are applied one by one, and
/// the message says exactly how many were saved if one fails.
pub async fn save(b: &MongoBackend, database: &str, collection: &str, changes: &Changes) -> Result<usize, String> {
    let planned = plan(b, database, collection, changes).await?;
    let coll = b.client.database(database).collection::<Document>(collection);
    let total = planned.len();
    for (done, p) in planned.into_iter().enumerate() {
        let result: Result<(), String> = match p.action {
            Action::Delete(id) => match coll.delete_one(doc! { "_id": id }).await {
                Ok(r) if r.deleted_count == 1 => Ok(()),
                Ok(_) => Err("it no longer exists".into()),
                Err(e) => Err(err(e)),
            },
            Action::Update(id, update) => match coll.update_one(doc! { "_id": id }, update).await {
                Ok(r) if r.matched_count == 1 => Ok(()),
                Ok(_) => Err("it no longer exists".into()),
                Err(e) => Err(err(e)),
            },
            Action::Insert(d) => coll.insert_one(d).await.map(|_| ()).map_err(err),
        };
        if let Err(e) = result {
            return Err(if done == 0 {
                format!("Nothing was saved. The {} failed: {e}", p.label)
            } else {
                format!("Saved {done} of {total} changes, then the {} failed: {e}. The saved ones stay saved (MongoDB has no transaction here); reload to see the current data.", p.label)
            });
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_building() {
        let mut c = Connection { engine: Engine::Mongodb, host: "db.example.com".into(), user: "app user".into(), ..Connection::default() };
        let s = Secrets { password: Some("p@ss:w/rd".into()), ..Secrets::default() };
        let u = build_uri(&c, &s, None);
        assert!(u.starts_with("mongodb://app%20user:p%40ss%3Aw%2Frd@db.example.com:27017/?"), "{u}");
        c.options.insert("srv".into(), "true".into());
        c.options.insert("authSource".into(), "admin".into());
        c.ssl.mode = SslMode::Required;
        let u = build_uri(&c, &s, None);
        assert!(u.starts_with("mongodb+srv://app%20user:p%40ss%3Aw%2Frd@db.example.com/?"), "{u}");
        assert!(u.contains("authSource=admin") && u.contains("tls=true") && u.contains("tlsAllowInvalidCertificates=true"));
        let t = build_uri(&c, &s, Some(50123));
        assert!(t.starts_with("mongodb://") && t.contains("@127.0.0.1:50123/") && t.contains("directConnection=true"));
    }

    #[test]
    fn rows_from_documents() {
        let docs = vec![
            doc! { "name": "a", "_id": bson::oid::ObjectId::parse_str("507f1f77bcf86cd799439011").unwrap(), "n": 1, "tags": ["x"] },
            doc! { "_id": 2, "name": "b", "extra": { "k": true }, "n": 2.5 },
        ];
        let rs = documents_to_result(docs, false);
        let names: Vec<_> = rs.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["_id", "name", "n", "tags", "extra"]);
        assert!(rs.columns[2].numeric);
        let rows = rs.rows(0, 2);
        assert_eq!(rows[0][0], Some("ObjectId('507f1f77bcf86cd799439011')"));
        assert_eq!(rows[0][3], Some("[\"x\"]"));
        assert_eq!(rows[1][4], Some("{\"k\":true}"));
        assert_eq!(rows[1][3], None, "missing field is empty");
    }

    #[test]
    fn typed_conversion() {
        assert_eq!(convert("42", "int", "n").unwrap(), Bson::Int32(42));
        assert_eq!(convert("4.5", "double", "n").unwrap(), Bson::Double(4.5));
        assert!(convert("abc", "int", "n").is_err());
        assert!(matches!(convert("2024-01-31T10:00:00Z", "date", "d").unwrap(), Bson::DateTime(_)));
        assert!(matches!(convert("{ a: [1, 2] }", "object", "o").unwrap(), Bson::Document(_)));
        assert_eq!(convert("hello", "string", "s").unwrap(), Bson::String("hello".into()));
        assert!(matches!(convert("507f1f77bcf86cd799439011", "objectId", "r").unwrap(), Bson::ObjectId(_)));
        assert_eq!(key_candidates("12").len(), 4);
    }
}
