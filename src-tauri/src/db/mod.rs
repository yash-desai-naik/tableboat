pub mod edit;
pub mod export;
pub mod meta;
pub mod mongo;
pub mod mongo_doc;
pub mod mongo_shell;
pub mod mysql;
pub mod query;
pub mod results;
pub mod split;
pub mod sqlite;
pub mod ssh;

use crate::conn::model::{Connection, Engine, Secrets};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerInfo {
    /// What the server turned out to be (a "mysql" connection may be MariaDB).
    pub engine: Option<Engine>,
    pub product: String,
    /// Short version, e.g. "8.0.42".
    pub version: String,
    pub full_version: String,
    /// The database selected after connecting, if any.
    pub database: Option<String>,
    pub tls: bool,
    pub ssh: bool,
}

impl ServerInfo {
    fn with_ssh(mut self, ssh: bool) -> Self {
        self.ssh = ssh;
        self
    }
}

/// A database connection. Cloning is cheap (a pool handle or a shared SQLite handle),
/// so commands take a clone and never hold a lock while a query runs.
#[derive(Clone)]
pub enum Backend {
    Mysql(mysql::MysqlBackend),
    Sqlite(Arc<sqlite::SqliteBackend>),
    Mongo(mongo::MongoBackend),
}

impl Backend {
    pub async fn close(&self) {
        match self {
            Backend::Mysql(b) => b.close().await,
            Backend::Sqlite(_) => {}
            Backend::Mongo(b) => b.client.clone().shutdown().await,
        }
    }
}

/// What commands work with: the connection, its backend and its metadata cache.
#[derive(Clone)]
pub struct SessionHandle {
    pub connection: Connection,
    pub info: ServerInfo,
    pub backend: Backend,
    pub meta: Arc<Mutex<meta::MetaCache>>,
}

struct SessionEntry {
    /// Label of the window that owns it; closed together with that window.
    window: String,
    handle: SessionHandle,
    /// Dropping it closes the SSH tunnel.
    tunnel: Option<ssh::Tunnel>,
}

/// Connect (opening an SSH tunnel first when configured).
pub async fn open(
    conn: &Connection,
    secrets: &Secrets,
    known_hosts: PathBuf,
) -> Result<(Backend, ServerInfo, Option<ssh::Tunnel>), String> {
    match conn.engine {
        Engine::Mysql | Engine::Mariadb => {
            if conn.host.trim().is_empty() {
                return Err("Enter the host.".into());
            }
            let port = conn.effective_port();
            let tunnel = if conn.ssh.enabled {
                Some(ssh::open(&conn.ssh, secrets, conn.host.trim(), port, known_hosts).await?)
            } else {
                None
            };
            let (host, port) = match &tunnel {
                Some(t) => ("127.0.0.1".to_string(), t.local_port),
                None => (conn.host.trim().to_string(), port),
            };
            let (backend, info) = mysql::MysqlBackend::connect(conn, secrets, &host, port).await?;
            let ssh = tunnel.is_some();
            Ok((Backend::Mysql(backend), info.with_ssh(ssh), tunnel))
        }
        Engine::Sqlite => {
            let conn = conn.clone();
            let (backend, info) = tokio::task::spawn_blocking(move || sqlite::SqliteBackend::open(&conn))
                .await
                .map_err(|e| e.to_string())??;
            Ok((Backend::Sqlite(Arc::new(backend)), info, None))
        }
        Engine::Mongodb => {
            let srv = conn.options.get("srv").map(|v| v == "true").unwrap_or(false);
            let tunnel = if conn.ssh.enabled {
                if srv || conn.host.contains(',') {
                    return Err("An SSH tunnel works with a single MongoDB host, not mongodb+srv or a host list.".into());
                }
                Some(ssh::open(&conn.ssh, secrets, conn.host.trim(), conn.effective_port(), known_hosts).await?)
            } else {
                None
            };
            let (backend, info) = mongo::connect(conn, secrets, tunnel.as_ref().map(|t| t.local_port)).await?;
            let ssh = tunnel.is_some();
            Ok((Backend::Mongo(backend), info.with_ssh(ssh), tunnel))
        }
        Engine::Postgres => Err(format!(
            "{} support arrives in a later step. MySQL, MariaDB and SQLite work now.",
            conn.engine.label()
        )),
    }
}

#[cfg(test)]
mod live_tests {
    //! Run against a real server:
    //!   $env:TABLEBOAT_TEST_MYSQL_URL = "mysql://user:pass@127.0.0.1:3306/db"; cargo test live_tests -- --nocapture
    //! Without the variable the MySQL tests do nothing. Never put real credentials in this file.
    use super::*;
    use crate::conn::url;

    fn known_hosts() -> PathBuf {
        std::env::temp_dir().join("tableboat-test-known_hosts")
    }

    fn test_url() -> Option<String> {
        std::env::var("TABLEBOAT_TEST_MYSQL_URL").ok().filter(|u| !u.is_empty())
    }

    #[tokio::test]
    async fn mysql_connects_and_reports_server() {
        let Some(u) = test_url() else { return };
        let (conn, secrets) = url::parse(&u).unwrap();
        let (backend, info, tunnel) = open(&conn, &secrets, known_hosts()).await.expect("connect");
        println!("connected: {info:?}");
        assert!(info.version.chars().next().unwrap().is_ascii_digit());
        assert!(matches!(info.engine, Some(Engine::Mysql) | Some(Engine::Mariadb)));
        if let Backend::Mysql(b) = backend {
            b.close().await;
        }
        drop(tunnel);
    }

    #[tokio::test]
    async fn mysql_metadata() {
        let Some(u) = test_url() else { return };
        let (conn, secrets) = url::parse(&u).unwrap();
        let (backend, info, tunnel) = open(&conn, &secrets, known_hosts()).await.expect("connect");
        let session = SessionHandle {
            connection: conn.clone(),
            info,
            backend: backend.clone(),
            meta: Arc::new(Mutex::new(meta::MetaCache::default())),
        };
        let start = std::time::Instant::now();
        let dbs = meta::databases(&session, false).await.expect("databases");
        println!("databases ({:?}): {dbs:?}", start.elapsed());
        assert!(dbs.contains(&conn.database));

        let start = std::time::Instant::now();
        let objects = meta::objects(&session, &conn.database, false).await.expect("objects");
        let count = |k| objects.iter().filter(|o| o.kind == k).count();
        println!(
            "objects in {} ({:?}): {} tables, {} views, {} functions, {} procedures, {} triggers",
            conn.database,
            start.elapsed(),
            count(meta::ObjectKind::Table),
            count(meta::ObjectKind::View),
            count(meta::ObjectKind::Function),
            count(meta::ObjectKind::Procedure),
            count(meta::ObjectKind::Trigger)
        );
        assert!(!objects.is_empty());

        let start = std::time::Instant::now();
        let hits = meta::search_session(&session, "usr", None, meta::SortBy::Score, 5).await.expect("search");
        println!(
            "search 'usr' across all databases ({:?}): {:?}",
            start.elapsed(),
            hits.iter().map(|h| format!("{}.{}", h.object.database, h.object.name)).collect::<Vec<_>>()
        );

        for kind in [meta::ObjectKind::Table, meta::ObjectKind::View, meta::ObjectKind::Trigger] {
            if let Some(o) = objects.iter().find(|o| o.kind == kind) {
                let ddl = meta::ddl(&session, &o.database, kind, &o.name).await.expect("ddl");
                println!("{kind:?} {}: {}", o.name, ddl.lines().next().unwrap_or(""));
                assert!(ddl.to_uppercase().contains("CREATE"));
            }
        }
        backend.close().await;
        drop(tunnel);
    }

    fn handle(conn: &Connection, info: ServerInfo, backend: &Backend) -> SessionHandle {
        SessionHandle {
            connection: conn.clone(),
            info,
            backend: backend.clone(),
            meta: Arc::new(Mutex::new(meta::MetaCache::default())),
        }
    }

    fn opts(db: &str, max_rows: usize) -> query::RunOptions {
        query::RunOptions { database: Some(db.into()), max_rows, use_pool: false, stop_on_error: false }
    }

    /// Read-only queries against the test server.
    #[tokio::test]
    async fn mysql_queries() {
        let Some(u) = test_url() else { return };
        let (conn, secrets) = url::parse(&u).unwrap();
        let (backend, info, tunnel) = open(&conn, &secrets, known_hosts()).await.expect("connect");
        let session = handle(&conn, info, &backend);
        let runs = query::Runs::default();
        let db = conn.database.clone();

        // Several statements, one failing in the middle: the rest still run.
        let out = query::run(
            &session,
            &runs,
            "r1",
            vec![
                "SELECT 1 AS one, 'x' AS two, NULL AS three, 12.50 AS dec_val, CAST(18446744073709551615 AS UNSIGNED) AS big".into(),
                "SELECT * FROM table_that_does_not_exist".into(),
                "SET @tb_test = 42".into(),
            ],
            opts(&db, 1000),
        )
        .await
        .unwrap();
        assert_eq!(out.len(), 3);
        let first = out[0].result.as_ref().expect("rows");
        println!("columns: {:?}", first.columns.iter().map(|c| format!("{}:{}", c.name, c.type_name)).collect::<Vec<_>>());
        let row = &first.rows(0, 1)[0];
        assert_eq!(row[0], Some("1"));
        assert_eq!(row[2], None, "NULL stays NULL");
        assert_eq!(row[3], Some("12.50"), "decimal text kept exactly");
        assert_eq!(row[4], Some("18446744073709551615"));
        println!("error message: {}", out[1].error.as_deref().unwrap());
        assert!(out[1].error.as_deref().unwrap().contains("1146"));
        assert!(out[2].error.is_none());

        // Session state carries over on the editor connection.
        let out = query::run(&session, &runs, "r2", vec!["SELECT @tb_test".into()], opts(&db, 10)).await.unwrap();
        assert_eq!(out[0].result.as_ref().unwrap().rows(0, 1)[0][0], Some("42"));

        // Row cap: stop early and leave the connection usable.
        let big = meta::objects(&session, &db, false)
            .await
            .unwrap()
            .into_iter()
            .filter(|o| o.kind == meta::ObjectKind::Table)
            .max_by_key(|o| o.rows.unwrap_or(0))
            .unwrap();
        let started = std::time::Instant::now();
        let out = query::run(
            &session,
            &runs,
            "r3",
            vec![format!("SELECT * FROM {}", meta::quote_mysql(&big.name)), "SELECT 2".into()],
            opts(&db, 500),
        )
        .await
        .unwrap();
        let rs = out[0].result.as_ref().unwrap();
        println!(
            "largest table {} (~{} rows): got {} rows, truncated={}, {:?}",
            big.name,
            big.rows.unwrap_or(0),
            rs.rows(0, usize::MAX).len(),
            rs.truncated,
            started.elapsed()
        );
        assert_eq!(rs.rows(0, usize::MAX).len().min(500), rs.rows(0, usize::MAX).len());
        assert_eq!(out[1].result.as_ref().unwrap().rows(0, 1)[0][0], Some("2"), "connection still usable");

        // Cancel a slow query.
        let runs = Arc::new(runs);
        let r2 = runs.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            r2.cancel("r4").await;
        });
        let started = std::time::Instant::now();
        let out = query::run(&session, &runs, "r4", vec!["SELECT SLEEP(10)".into(), "SELECT 3".into()], opts(&db, 10))
            .await
            .unwrap();
        println!("cancelled after {:?}: {:?}", started.elapsed(), out.iter().map(|o| (o.cancelled, o.error.clone())).collect::<Vec<_>>());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        let out = query::run(&session, &runs, "r5", vec!["SELECT 4".into()], opts(&db, 10)).await.unwrap();
        assert_eq!(out[0].result.as_ref().unwrap().rows(0, 1)[0][0], Some("4"), "usable after cancel");

        backend.close().await;
        drop(tunnel);
    }

    #[tokio::test]
    async fn sqlite_queries() {
        let path = std::env::temp_dir().join(format!("tableboat-q-{}.sqlite", uuid::Uuid::new_v4()));
        let p = path.to_string_lossy().into_owned();
        sqlite::create_file(&p).unwrap();
        let conn = Connection { engine: Engine::Sqlite, file: Some(p), ..Connection::default() };
        let (backend, info, _) = open(&conn, &Secrets::default(), known_hosts()).await.unwrap();
        let session = handle(&conn, info, &backend);
        let runs = query::Runs::default();
        let o = query::RunOptions { database: None, max_rows: 3, use_pool: false, stop_on_error: false };
        let out = query::run(
            &session,
            &runs,
            "s1",
            vec![
                "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT, score REAL, data BLOB)".into(),
                "INSERT INTO t (name, score, data) VALUES ('a', 1.5, x'00ff'), ('b', NULL, NULL), ('c', 3, NULL), ('d', 4, NULL)".into(),
                "SELECT * FROM t ORDER BY id".into(),
                "SELECT nonsense FROM".into(),
            ],
            o,
        )
        .await
        .unwrap();
        assert_eq!(out[1].affected_rows, Some(4));
        let rs = out[2].result.as_ref().unwrap();
        assert!(rs.truncated, "capped at 3 rows");
        assert_eq!(rs.rows(0, 10).len(), 3);
        assert_eq!(rs.rows(0, 1)[0][3], Some("0x00FF"));
        assert!(rs.columns[2].numeric);
        assert!(out[3].error.is_some());
        drop(session);
        drop(backend);
        let _ = std::fs::remove_file(&path);
    }

    /// Editing round trip on a scratch SQLite file: update, duplicate, delete, rollback.
    #[tokio::test]
    async fn sqlite_editing() {
        let path = std::env::temp_dir().join(format!("tableboat-e-{}.sqlite", uuid::Uuid::new_v4()));
        let p = path.to_string_lossy().into_owned();
        sqlite::create_file(&p).unwrap();
        let conn = Connection { engine: Engine::Sqlite, file: Some(p), ..Connection::default() };
        let (backend, info, _) = open(&conn, &Secrets::default(), known_hosts()).await.unwrap();
        let session = handle(&conn, info, &backend);
        let runs = query::Runs::default();
        let o = || query::RunOptions { database: None, max_rows: 100, use_pool: false, stop_on_error: true };
        query::run(
            &session,
            &runs,
            "e0",
            vec![
                "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT, full TEXT GENERATED ALWAYS AS (name || '!') VIRTUAL)".into(),
                "INSERT INTO people (name, note) VALUES ('a', 'x'), ('b', NULL), ('c', 'z')".into(),
            ],
            o(),
        )
        .await
        .unwrap();

        let cols = meta::table_columns(&session, "main", "people").await.unwrap();
        assert!(cols.iter().any(|c| c.name == "id" && c.primary_key && c.auto_increment));
        assert!(cols.iter().any(|c| c.name == "full" && c.generated));

        let changes: edit::Changes = serde_json::from_str(
            r#"{
              "updates": [{"key": [{"column": "id", "value": "1"}], "set": [{"column": "name", "value": {"kind": "text", "value": "a'; DROP TABLE people; --"}}]}],
              "inserts": [{"values": [{"column": "id", "value": {"kind": "default"}}, {"column": "name", "value": {"kind": "text", "value": "b"}}, {"column": "note", "value": {"kind": "null"}}, {"column": "full", "value": {"kind": "default"}}]}],
              "deletes": [{"key": [{"column": "id", "value": "3"}]}]
            }"#,
        )
        .unwrap();
        let preview = edit::preview(&session, "main", "people", &changes).await.unwrap();
        println!("preview: {preview:#?}");
        assert_eq!(edit::save(&session, "main", "people", &changes).await.unwrap(), 3);

        let out = query::run(&session, &runs, "e1", vec!["SELECT id, name, note, full FROM people ORDER BY id".into()], o()).await.unwrap();
        let rows = out[0].result.as_ref().unwrap().rows(0, 10);
        println!("after save: {rows:?}");
        assert_eq!(rows.len(), 3, "row 3 deleted, one duplicate added");
        assert_eq!(rows[0][1], Some("a'; DROP TABLE people; --"), "stored as text, not run");
        assert_eq!(rows[2][1], Some("b"));
        assert_eq!(rows[2][3], Some("b!"), "generated column computed by SQLite");

        // A failing statement rolls back everything before it.
        let bad: edit::Changes = serde_json::from_str(
            r#"{
              "updates": [{"key": [{"column": "id", "value": "1"}], "set": [{"column": "name", "value": {"kind": "text", "value": "changed"}}]},
                          {"key": [{"column": "id", "value": "999"}], "set": [{"column": "name", "value": {"kind": "text", "value": "ghost"}}]}]
            }"#,
        )
        .unwrap();
        let err = edit::save(&session, "main", "people", &bad).await.unwrap_err();
        println!("rollback: {err}");
        assert!(err.contains("Nothing was saved") && err.contains("0 rows"));
        let out = query::run(&session, &runs, "e2", vec!["SELECT name FROM people WHERE id = 1".into()], o()).await.unwrap();
        assert_eq!(out[0].result.as_ref().unwrap().rows(0, 1)[0][0], Some("a'; DROP TABLE people; --"), "first update rolled back");

        drop(session);
        drop(backend);
        let _ = std::fs::remove_file(&path);
    }

    /// MySQL: read a table's columns and build a preview. Nothing is written.
    #[tokio::test]
    async fn mysql_edit_preview_only() {
        let Some(u) = test_url() else { return };
        let (conn, secrets) = url::parse(&u).unwrap();
        let (backend, info, tunnel) = open(&conn, &secrets, known_hosts()).await.expect("connect");
        let session = handle(&conn, info, &backend);
        let table = meta::objects(&session, &conn.database, false)
            .await
            .unwrap()
            .into_iter()
            .find(|o| o.kind == meta::ObjectKind::Table)
            .unwrap();
        let cols = meta::table_columns(&session, &conn.database, &table.name).await.unwrap();
        println!(
            "{}: {}",
            table.name,
            cols.iter()
                .map(|c| format!("{}{}{}", c.name, if c.primary_key { "(pk)" } else { "" }, if c.auto_increment { "(ai)" } else { "" }))
                .collect::<Vec<_>>()
                .join(", ")
        );
        if let Some(pk) = cols.iter().find(|c| c.primary_key) {
            let other = cols.iter().find(|c| !c.primary_key && !c.generated).unwrap();
            let changes: edit::Changes = serde_json::from_value(serde_json::json!({
                "updates": [{"key": [{"column": pk.name, "value": "1"}], "set": [{"column": other.name, "value": {"kind": "null"}}]}]
            }))
            .unwrap();
            let preview = edit::preview(&session, &conn.database, &table.name, &changes).await.unwrap();
            println!("preview: {preview:?}");
            assert!(preview[0].starts_with("UPDATE `") && preview[0].ends_with("LIMIT 1;"));
        }
        backend.close().await;
        drop(tunnel);
    }

    /// MongoDB: reads from the test server, writes only in a scratch database that's dropped.
    #[tokio::test]
    async fn mongo_live() {
        let Some(u) = std::env::var("TABLEBOAT_TEST_MONGO_URL").ok().filter(|u| !u.is_empty()) else { return };
        let (conn, secrets) = url::parse(&u).unwrap();
        let (backend, info, _t) = open(&conn, &secrets, known_hosts()).await.expect("connect");
        println!("connected: {} {}", info.product, info.version);
        let session = handle(&conn, info, &backend);
        let runs = query::Runs::default();

        let dbs = meta::databases(&session, false).await.unwrap();
        println!("databases: {dbs:?}");
        if !conn.database.is_empty() {
            let objs = meta::objects(&session, &conn.database, false).await.unwrap();
            println!("{} collections in {}", objs.len(), conn.database);
            if let Some(first) = objs.first() {
                let out = query::run(
                    &session,
                    &runs,
                    "m0",
                    vec![format!("db.getCollection({}).find({{}}).limit(5)", serde_json::to_string(&first.name).unwrap())],
                    query::RunOptions { database: Some(conn.database.clone()), max_rows: 3, use_pool: true, stop_on_error: true },
                )
                .await
                .unwrap();
                let rs = out[0].result.as_ref().expect("documents");
                println!("read {} docs from {} (truncated={}), columns: {:?}", rs.rows(0, 10).len(), first.name, rs.truncated, rs.columns.iter().map(|c| format!("{}:{}", c.name, c.type_name)).take(6).collect::<Vec<_>>());
                assert!(rs.truncated, "row cap stops at 3");
            }
        }

        // Writes in a scratch database only.
        let scratch = "tableboat_scratch";
        let o = || query::RunOptions { database: Some(scratch.into()), max_rows: 100, use_pool: true, stop_on_error: true };
        let out = query::run(
            &session,
            &runs,
            "m1",
            vec![
                "db.people.insertMany([{ name: 'a', n: 1, born: ISODate('2000-01-01T00:00:00Z'), tags: ['x'] }, { name: 'b', n: 2 }, { name: 'c', n: 3 }])".into(),
                "db.people.find({ n: { $gte: 2 } }).sort({ n: -1 })".into(),
                "db.people.countDocuments({})".into(),
                "db.people.aggregate([{ $group: { _id: null, total: { $sum: '$n' } } }])".into(),
                "db.people.updateOne({ name: 'b' }, { $set: { n: 20 } })".into(),
            ],
            o(),
        )
        .await
        .unwrap();
        for x in &out {
            println!("{} → err={:?} affected={:?} info={:?} rows={:?}", x.sql.chars().take(50).collect::<String>(), x.error, x.affected_rows, x.info, x.result.as_ref().map(|r| r.rows(0, 3)));
        }
        assert!(out.iter().all(|x| x.error.is_none()));
        assert_eq!(out[1].result.as_ref().unwrap().rows(0, 1)[0][2], Some("3"), "sorted n desc");

        // Grid editing: keep types, delete, duplicate.
        let first = query::run(&session, &runs, "m2", vec!["db.people.find({ name: 'a' })".into()], o()).await.unwrap();
        let id = first[0].result.as_ref().unwrap().rows(0, 1)[0][0].unwrap().to_string();
        let changes: edit::Changes = serde_json::from_value(serde_json::json!({
            "updates": [{ "key": [{ "column": "_id", "value": id }], "set": [
                { "column": "n", "value": { "kind": "text", "value": "41" } },
                { "column": "born", "value": { "kind": "text", "value": "2001-02-03T04:05:06Z" } },
                { "column": "tags", "value": { "kind": "text", "value": "['x', 'y']" } }
            ] }],
            "inserts": [{ "values": [
                { "column": "_id", "value": { "kind": "default" } },
                { "column": "name", "value": { "kind": "text", "value": "a copy" } },
                { "column": "n", "value": { "kind": "text", "value": "7" } }
            ] }]
        }))
        .unwrap();
        println!("preview: {:#?}", edit::preview(&session, scratch, "people", &changes).await.unwrap());
        assert_eq!(edit::save(&session, scratch, "people", &changes).await.unwrap(), 2);
        let check = query::run(
            &session,
            &runs,
            "m3",
            vec!["db.people.aggregate([{ $project: { name: 1, nType: { $type: '$n' }, bornType: { $type: '$born' }, tags: 1 } }, { $sort: { name: 1 } }])".into()],
            o(),
        )
        .await
        .unwrap();
        let rows = check[0].result.as_ref().unwrap().rows(0, 10);
        println!("after save: {rows:?}");
        let a = rows.iter().find(|r| r.iter().any(|c| *c == Some("a"))).unwrap();
        assert!(a.contains(&Some("int")), "n stayed an int: {a:?}");
        assert!(a.contains(&Some("date")), "born stayed a date: {a:?}");
        assert!(rows.iter().any(|r| r.contains(&Some("a copy"))), "duplicate inserted");

        // Clean up.
        let out = query::run(&session, &runs, "m4", vec!["db.runCommand({ dropDatabase: 1 })".into()], o()).await.unwrap();
        assert!(out[0].error.is_none());
        backend.close().await;
    }

    #[tokio::test]
    async fn mysql_wrong_password_is_explained() {
        let Some(u) = test_url() else { return };
        let (conn, mut secrets) = url::parse(&u).unwrap();
        secrets.password = Some("definitely-not-the-password".into());
        let err = open(&conn, &secrets, known_hosts()).await.err().expect("must fail");
        println!("wrong password: {err}");
        assert!(err.contains("Access denied"), "{err}");
    }

    #[tokio::test]
    async fn mysql_closed_port_is_explained() {
        let (mut conn, secrets) = url::parse("mysql://root@127.0.0.1:1/x").unwrap();
        conn.ssl.mode = crate::conn::model::SslMode::Disabled;
        let err = open(&conn, &secrets, known_hosts()).await.err().expect("must fail");
        println!("closed port: {err}");
        assert!(err.contains("127.0.0.1:1"), "{err}");
    }

    #[tokio::test]
    async fn sqlite_create_and_open() {
        let path = std::env::temp_dir().join(format!("tableboat-test-{}.sqlite", uuid::Uuid::new_v4()));
        let p = path.to_string_lossy().into_owned();
        sqlite::create_file(&p).expect("create");
        assert!(sqlite::create_file(&p).is_err(), "must refuse to overwrite");
        let conn = Connection { engine: Engine::Sqlite, file: Some(p.clone()), ..Connection::default() };
        let (backend, info, _) = open(&conn, &Secrets::default(), known_hosts()).await.expect("open");
        println!("sqlite: {info:?}");
        assert_eq!(info.product, "SQLite");
        drop(backend);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn credential_manager_round_trip() {
        use crate::conn::secrets;
        secrets::init().unwrap();
        let id = format!("test-{}", uuid::Uuid::new_v4());
        let pw = "p@ss w:rd/ü✓";
        secrets::set(&id, secrets::PASSWORD, pw).unwrap();
        assert_eq!(secrets::get(&id, secrets::PASSWORD).unwrap().as_deref(), Some(pw));
        secrets::delete_all(&id).unwrap();
        assert_eq!(secrets::get(&id, secrets::PASSWORD).unwrap(), None);
    }
}

/// Every open session, across all windows, keyed by session id.
#[derive(Default)]
pub struct Sessions {
    map: Mutex<HashMap<String, SessionEntry>>,
}

impl Sessions {
    pub async fn insert(
        &self,
        window: String,
        connection: Connection,
        info: ServerInfo,
        backend: Backend,
        tunnel: Option<ssh::Tunnel>,
    ) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let handle = SessionHandle { connection, info, backend, meta: Arc::new(Mutex::new(meta::MetaCache::default())) };
        self.map.lock().await.insert(id.clone(), SessionEntry { window, handle, tunnel });
        id
    }

    pub async fn get(&self, id: &str) -> Result<SessionHandle, String> {
        self.map
            .lock()
            .await
            .get(id)
            .map(|e| e.handle.clone())
            .ok_or_else(|| "This connection was closed. Connect again.".to_string())
    }

    pub async fn close(&self, id: &str) {
        let entry = self.map.lock().await.remove(id);
        if let Some(entry) = entry {
            entry.handle.backend.close().await;
            drop(entry.tunnel);
        }
    }

    pub async fn close_window(&self, window: &str) {
        let ids: Vec<String> = self
            .map
            .lock()
            .await
            .iter()
            .filter(|(_, e)| e.window == window)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.close(&id).await;
        }
    }
}
