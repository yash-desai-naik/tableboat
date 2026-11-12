//! What each `tb` command does (except `tb ask`, in ask.rs).

use crate::cli::{AddArgs, DescribeArgs, DumpArgs, EditArgs, ExportArgs, Fmt, Kind, QueryArgs, SettingsCmd, TablesArgs};
use crate::{out, term};
use std::process::ExitCode;
use tableboat_core::conn::model::{Connection, Engine, Secrets, SslMode};
use tableboat_core::conn::store::ConnectionStore;
use tableboat_core::conn::{ops, secrets, url};
use tableboat_core::db::export;
use tableboat_core::db::meta::ObjectKind;
use tableboat_core::db::results::{self, ResultSet};
use tableboat_core::db::split::{self, Dialect};
use tableboat_core::db::{self, SessionHandle, Sessions};
use tableboat_core::{claude, paths, prefs};

/// Everything a command needs: saved connections, open sessions, query runs, settings.
pub struct Ctx {
    pub store: ConnectionStore,
    pub sessions: Sessions,
    pub runs: db::query::Runs,
    pub settings: prefs::Settings,
}

/// Sessions opened by `tb` belong to this owner (closed together at the end).
pub const OWNER: &str = "tb";

impl Ctx {
    pub fn load() -> Result<Self, String> {
        secrets::init()?;
        let dir = paths::data_dir()?;
        Ok(Self {
            store: ConnectionStore::load(&dir)?,
            sessions: Sessions::default(),
            runs: db::query::Runs::default(),
            settings: prefs::load(&prefs::path()?),
        })
    }

    pub async fn close(&self) {
        self.sessions.close_window(OWNER).await;
    }

    /// A saved connection by id, name, or any unique part of its name.
    pub fn find(&self, query: &str) -> Result<Connection, String> {
        ops::find(&self.store, query).map_err(|e| {
            if self.store.list().is_empty() { format!("{e} Add one with `tb conn add <url>`.") } else { e }
        })
    }

    /// Connect (asking for the password when the connection doesn't save it).
    pub async fn open(&self, conn: &Connection) -> Result<SessionHandle, String> {
        Ok(open_in(&self.sessions, conn).await?.1)
    }
}

/// Connect into the given session list (owner `tb`); returns the session id and handle.
pub async fn open_in(sessions: &Sessions, conn: &Connection) -> Result<(String, SessionHandle), String> {
    let mut secrets = secrets::load(&conn.id)?;
    if !conn.save_password && conn.engine != Engine::Sqlite && secrets.password.is_none() {
        secrets.password = Some(match std::env::var("TABLEBOAT_PASSWORD") {
            Ok(p) => p,
            Err(_) => term::password(&format!("Password for {}: ", conn.display_name()))
                .ok_or("This connection asks for its password every time; set TABLEBOAT_PASSWORD when there's no console.")?,
        });
    }
    let (backend, info, tunnel) = db::open(conn, &secrets, ops::known_hosts()?)
        .await
        .map_err(|e| format!("{}: {e}", conn.display_name()))?;
    let id = sessions.insert(OWNER.into(), conn.clone(), info, backend, tunnel).await;
    let handle = sessions.get(&id).await?;
    Ok((id, handle))
}

fn target(c: &Connection) -> String {
    match c.engine {
        Engine::Sqlite => c.file.clone().unwrap_or_default(),
        _ => {
            let user = if c.user.is_empty() { String::new() } else { format!("{}@", c.user) };
            let db = if c.database.is_empty() { String::new() } else { format!("/{}", c.database) };
            let ssh = if c.ssh.enabled { format!(" via ssh {}@{}", c.ssh.user, c.ssh.host) } else { String::new() };
            format!("{user}{}:{}{db}{ssh}", c.host, c.effective_port())
        }
    }
}

pub fn engine_dialect(engine: Engine) -> (Dialect, results::Dialect) {
    match engine {
        Engine::Sqlite => (Dialect::Sqlite, results::Dialect::Sqlite),
        Engine::Mongodb => (Dialect::Mongo, results::Dialect::Mysql),
        _ => (Dialect::Mysql, results::Dialect::Mysql),
    }
}

/// The database to use: the one asked for, else the connection's default (SQLite: main).
fn database(conn: &Connection, asked: Option<&str>) -> Option<String> {
    asked
        .map(str::to_string)
        .or_else(|| Some(conn.database.clone()).filter(|d| !d.is_empty()))
        .or_else(|| (conn.engine == Engine::Sqlite).then(|| "main".to_string()))
}

async fn need_database(conn: &Connection, session: &SessionHandle, asked: Option<&str>) -> Result<String, String> {
    if let Some(d) = database(conn, asked) {
        return Ok(d);
    }
    let list = db::meta::databases(session, false).await.unwrap_or_default();
    Err(format!("{} has no default database: pass -d <database>. Databases: {}", conn.display_name(), list.join(", ")))
}

fn ok() -> ExitCode {
    ExitCode::SUCCESS
}

// ---------- connections ----------

pub fn conn_list(ctx: &Ctx, format: Option<Fmt>) -> Result<ExitCode, String> {
    let mut all = ctx.store.list();
    all.sort_by_key(|c| c.display_name().to_lowercase());
    let rows = all
        .iter()
        .map(|c| vec![Some(c.display_name()), Some(c.engine.label().to_string()), Some(target(c)), c.tag.clone(), Some(c.id.clone())])
        .collect();
    let rs = out::list(&["name", "engine", "target", "tag", "id"], rows, &[]);
    out::print(&rs, out::pick(format), results::Dialect::Mysql, Some("connections"), false)?;
    Ok(ok())
}

pub fn conn_show(ctx: &Ctx, query: &str) -> Result<ExitCode, String> {
    let c = ctx.find(query)?;
    let s = ops::summary(c.clone());
    let yes_no = |b: bool| if b { "saved" } else { "not saved" };
    let mut lines = vec![
        ("name", c.display_name()),
        ("id", c.id.clone()),
        ("engine", c.engine.label().to_string()),
        ("target", target(&c)),
        ("tag", c.tag.clone().unwrap_or_default()),
        ("colour", c.color.clone()),
    ];
    if c.engine != Engine::Sqlite {
        lines.push(("ssl", format!("{:?}", c.ssl.mode).to_lowercase()));
        lines.push(("password", if c.save_password { yes_no(s.has_password).into() } else { "asked every time".into() }));
    }
    lines.push(("url", url::to_url(&c, &Secrets::default(), false)));
    if c.ssh.enabled {
        lines.push(("ssh", format!("{}@{}:{} ({:?})", c.ssh.user, c.ssh.host, c.ssh.port, c.ssh.auth).to_lowercase()));
    }
    if !c.options.is_empty() {
        lines.push(("options", c.options.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")));
    }
    lines.push((
        "claude project",
        claude::existing_project_folder(&c.id)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(made on the first Claude chat for it)".into()),
    ));
    for (k, v) in lines {
        println!("{:<15} {v}", term::dim(k));
    }
    Ok(ok())
}

pub async fn conn_add(ctx: &Ctx, a: AddArgs) -> Result<ExitCode, String> {
    let (mut conn, secrets) = url::parse(&a.url)?;
    if let Some(n) = a.name {
        conn.name = n;
    }
    if let Some(c) = a.color {
        conn.color = if c.starts_with('#') { c } else { format!("#{c}") };
    }
    if let Some(t) = a.tag {
        conn.tag = Some(t).filter(|t| !t.is_empty());
    }
    if a.ask_password {
        conn.save_password = false;
    }
    if a.test {
        let (backend, info, tunnel) = db::open(&conn, &secrets, ops::known_hosts()?).await?;
        backend.close().await;
        drop(tunnel);
        eprintln!("{} {} {}", term::green("✓"), info.product, info.version);
    }
    let saved = ops::save(&ctx.store, conn, secrets)?;
    println!("Saved \"{}\" ({}, {})", saved.connection.display_name(), saved.connection.engine.label(), target(&saved.connection));
    Ok(ok())
}

pub fn conn_edit(ctx: &Ctx, a: EditArgs) -> Result<ExitCode, String> {
    let mut c = ctx.find(&a.connection)?;
    let mut s = Secrets::default();
    if let Some(v) = a.name {
        c.name = v;
    }
    if let Some(v) = a.color {
        c.color = if v.starts_with('#') { v } else { format!("#{v}") };
    }
    if let Some(v) = a.tag {
        c.tag = Some(v).filter(|t| !t.is_empty());
    }
    if let Some(v) = a.host {
        c.host = v;
    }
    if let Some(v) = a.port {
        c.port = Some(v);
    }
    if let Some(v) = a.user {
        c.user = v;
    }
    if let Some(v) = a.database {
        c.database = v;
    }
    if let Some(v) = a.file {
        c.file = Some(v);
    }
    if let Some(v) = a.ssl_mode {
        c.ssl.mode = serde_json::from_value::<SslMode>(serde_json::Value::String(v.clone()))
            .map_err(|_| format!("Unknown SSL mode \"{v}\": use disabled, preferred, required, verify-ca or verify-identity."))?;
    }
    if a.ask_password {
        c.save_password = false;
    }
    if a.password {
        c.save_password = true;
        s.password = Some(term::password("New password: ").ok_or("No console to type the password in.")?);
    }
    let saved = ops::save(&ctx.store, c, s)?;
    println!("Saved \"{}\" ({})", saved.connection.display_name(), target(&saved.connection));
    Ok(ok())
}

pub fn conn_rm(ctx: &Ctx, query: &str, yes: bool) -> Result<ExitCode, String> {
    let c = ctx.find(query)?;
    if !yes {
        if !term::has_console() {
            return Err("Add --yes to delete without a console to confirm in.".into());
        }
        if !term::confirm(&format!("Delete \"{}\" and its saved passwords?", c.display_name())) {
            eprintln!("Kept.");
            return Ok(ExitCode::FAILURE);
        }
    }
    ops::delete(&ctx.store, &c.id)?;
    println!("Deleted \"{}\"", c.display_name());
    Ok(ok())
}

pub async fn conn_test(ctx: &Ctx, target_arg: &str) -> Result<ExitCode, String> {
    let (conn, secrets) = if target_arg.contains("://") {
        url::parse(target_arg)?
    } else {
        let c = ctx.find(target_arg)?;
        let s = secrets::load(&c.id)?;
        (c, s)
    };
    let started = std::time::Instant::now();
    let session = if secrets.password.is_none() && !conn.save_password && conn.engine != Engine::Sqlite && !conn.id.is_empty() {
        ctx.open(&conn).await?
    } else {
        let (backend, info, tunnel) = db::open(&conn, &secrets, ops::known_hosts()?).await?;
        let id = ctx.sessions.insert(OWNER.into(), conn.clone(), info, backend, tunnel).await;
        ctx.sessions.get(&id).await?
    };
    println!(
        "{} {} {} · {} · {:.0} ms",
        term::green("✓"),
        session.info.product,
        session.info.version,
        conn.display_name(),
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(ok())
}

pub fn conn_url(ctx: &Ctx, query: &str, with_passwords: bool) -> Result<ExitCode, String> {
    let c = ctx.find(query)?;
    println!("{}", ops::to_url(&ctx.store, &c.id, with_passwords)?);
    Ok(ok())
}

pub fn conn_duplicate(ctx: &Ctx, query: &str) -> Result<ExitCode, String> {
    let c = ctx.find(query)?;
    let copy = ops::duplicate(&ctx.store, &c.id)?;
    println!("Saved \"{}\"", copy.connection.display_name());
    Ok(ok())
}

pub fn conn_export(ctx: &Ctx, file: &str, with_passwords: bool) -> Result<ExitCode, String> {
    let n = ops::export_file(&ctx.store, std::path::Path::new(file), with_passwords)?;
    println!("Wrote {n} connection{} to {file}{}", if n == 1 { "" } else { "s" }, if with_passwords { " (with passwords: keep it private)" } else { "" });
    Ok(ok())
}

pub fn conn_import(ctx: &Ctx, file: &str) -> Result<ExitCode, String> {
    let (n, errors) = ops::import_file(&ctx.store, std::path::Path::new(file))?;
    println!("Imported {n} connection{}", if n == 1 { "" } else { "s" });
    for e in &errors {
        eprintln!("{} {e}", term::red("skipped:"));
    }
    Ok(if errors.is_empty() { ok() } else { ExitCode::FAILURE })
}

// ---------- browsing ----------

pub async fn databases(ctx: &Ctx, query: &str, format: Option<Fmt>) -> Result<ExitCode, String> {
    let c = ctx.find(query)?;
    let session = ctx.open(&c).await?;
    let list = db::meta::databases(&session, false).await?;
    let rows = list.into_iter().map(|d| vec![Some(d)]).collect();
    out::print(&out::list(&["database"], rows, &[]), out::pick(format), results::Dialect::Mysql, None, false)?;
    Ok(ok())
}

fn kind_label(kind: ObjectKind, engine: Engine) -> &'static str {
    match kind {
        ObjectKind::Table if engine == Engine::Mongodb => "collection",
        ObjectKind::Table => "table",
        ObjectKind::View => "view",
        ObjectKind::Function => "function",
        ObjectKind::Procedure => "procedure",
        ObjectKind::Trigger => "trigger",
    }
}

pub async fn tables(ctx: &Ctx, a: TablesArgs) -> Result<ExitCode, String> {
    let c = ctx.find(&a.connection)?;
    let session = ctx.open(&c).await?;
    let database = need_database(&c, &session, a.database.as_deref()).await?;
    let wanted = a.kind.map(|k| match k {
        Kind::Table => ObjectKind::Table,
        Kind::View => ObjectKind::View,
        Kind::Function => ObjectKind::Function,
        Kind::Procedure => ObjectKind::Procedure,
        Kind::Trigger => ObjectKind::Trigger,
    });
    let filter = a.filter.as_deref().map(str::to_lowercase);
    let objects = db::meta::objects(&session, &database, false).await?;
    let rows: Vec<Vec<Option<String>>> = objects
        .into_iter()
        .filter(|o| wanted.is_none_or(|k| o.kind == k))
        .filter(|o| filter.as_deref().is_none_or(|f| o.name.to_lowercase().contains(f)))
        .map(|o| {
            vec![
                Some(o.name),
                Some(kind_label(o.kind, c.engine).to_string()),
                o.rows.map(|r| r.to_string()),
                o.parent.or(o.comment).filter(|s| !s.is_empty()),
            ]
        })
        .collect();
    let n = rows.len();
    out::print(&out::list(&["name", "kind", "rows", "note"], rows, &[2]), out::pick(a.format), results::Dialect::Mysql, None, false)?;
    eprintln!("{}", term::dim(&format!("{n} object{} in {database}", if n == 1 { "" } else { "s" })));
    Ok(ok())
}

pub async fn describe(ctx: &Ctx, a: DescribeArgs) -> Result<ExitCode, String> {
    let c = ctx.find(&a.connection)?;
    let session = ctx.open(&c).await?;
    let database = need_database(&c, &session, a.database.as_deref()).await?;
    let cols = db::meta::table_columns(&session, &database, &a.table).await?;
    if cols.is_empty() {
        return Err(format!("No table or collection named \"{}\" in {database} (or it has no columns).", a.table));
    }
    let rows = cols
        .iter()
        .map(|col| {
            let mut extra = Vec::new();
            if col.auto_increment {
                extra.push("auto_increment");
            }
            if col.generated {
                extra.push("generated");
            }
            vec![
                Some(col.name.clone()),
                Some(col.type_name.clone()),
                Some(if col.nullable { "yes" } else { "no" }.to_string()),
                col.primary_key.then(|| "PRI".to_string()),
                col.default.clone(),
                Some(extra.join(", ")).filter(|s| !s.is_empty()),
            ]
        })
        .collect();
    let (_, rdialect) = engine_dialect(c.engine);
    out::print(&out::list(&["column", "type", "null", "key", "default", "extra"], rows, &[]), out::pick(a.format), rdialect, Some(&a.table), false)?;
    if a.ddl {
        // A view's definition when it's a view, else the table's.
        let kind = db::meta::objects(&session, &database, false)
            .await
            .ok()
            .and_then(|objs| objs.into_iter().find(|o| o.name == a.table).map(|o| o.kind))
            .unwrap_or(ObjectKind::Table);
        let ddl = db::meta::ddl(&session, &database, kind, &a.table).await?;
        println!("\n{ddl}");
    }
    Ok(ok())
}

// ---------- queries ----------

const READ_WORDS: [&str; 9] = ["select", "with", "show", "describe", "desc", "explain", "pragma", "values", "table"];
const WRITE_WORDS: [&str; 10] = ["insert", "update", "delete", "replace", "drop", "alter", "create", "truncate", "grant", "revoke"];

/// Could this statement change data? (Strings are ignored; anything unknown counts as a write.)
fn may_write(sql: &str, engine: Engine) -> bool {
    if engine == Engine::Mongodb {
        use tableboat_core::db::mongo_shell::{Command, parse};
        return match parse(sql) {
            Ok(Command::ShowDatabases | Command::ShowCollections | Command::CollectionNames | Command::Use(_)) => false,
            Ok(Command::Collection { op, .. }) => !op.is_read_only(),
            _ => true,
        };
    }
    let mut code = String::new();
    let mut quote: Option<char> = None;
    for ch in sql.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' || ch == '`' => quote = Some(ch),
            None => code.push(ch.to_ascii_lowercase()),
        }
    }
    let words: Vec<&str> = code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).filter(|w| !w.is_empty()).collect();
    match words.first() {
        Some(w) if READ_WORDS.contains(w) => words.iter().any(|w| WRITE_WORDS.contains(w)),
        _ => true,
    }
}

fn read_sql(a: &QueryArgs) -> Result<String, String> {
    if let Some(f) = &a.file {
        return std::fs::read_to_string(f).map_err(|e| format!("Could not read {f}: {e}"));
    }
    if !a.sql.is_empty() {
        return Ok(a.sql.join(" "));
    }
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return Err("Give the SQL as an argument, with --file, or pipe it in.".into());
    }
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

pub async fn query(ctx: &Ctx, a: QueryArgs) -> Result<ExitCode, String> {
    let c = ctx.find(&a.connection)?;
    let text = read_sql(&a)?;
    let (dialect, rdialect) = engine_dialect(c.engine);
    let statements = split::split(&text, dialect);
    if statements.is_empty() {
        return Err("Nothing to run.".into());
    }

    // Changing data on production needs a yes.
    let production = c.tag.as_deref().is_some_and(|t| t.eq_ignore_ascii_case("production"));
    if production && !a.read_only && !a.yes && statements.iter().any(|s| may_write(s, c.engine)) {
        if !term::has_console() {
            return Err(format!("\"{}\" is tagged production and this may change data: add --yes to run it.", c.display_name()));
        }
        if !term::confirm(&format!("{} \"{}\" is tagged production and this may change data. Run it?", term::red("!"), c.display_name())) {
            eprintln!("Not run.");
            return Ok(ExitCode::FAILURE);
        }
    }
    if a.read_only && c.engine == Engine::Mongodb {
        if let Some(bad) = statements.iter().find(|s| may_write(s, c.engine)) {
            return Err(format!("--read-only: this could change data: {bad}"));
        }
    }

    let session = ctx.open(&c).await?;
    let max_rows = if a.all { 50_000_000 } else { a.max_rows.unwrap_or(ctx.settings.max_rows as usize) };
    // --read-only: the database itself refuses writes.
    let (wrapped, skip_first, skip_last) = match (a.read_only, c.engine) {
        (true, Engine::Mysql | Engine::Mariadb) => {
            let mut v = vec!["START TRANSACTION READ ONLY".to_string()];
            v.extend(statements.iter().cloned());
            v.push("ROLLBACK".into());
            (v, true, true)
        }
        (true, Engine::Sqlite) => {
            let mut v = vec!["PRAGMA query_only = ON".to_string()];
            v.extend(statements.iter().cloned());
            v.push("PRAGMA query_only = OFF".into());
            (v, true, true)
        }
        _ => (statements.clone(), false, false),
    };
    let opts = db::query::RunOptions {
        database: database(&c, a.database.as_deref()).filter(|_| c.engine != Engine::Sqlite),
        max_rows,
        use_pool: false,
        stop_on_error: !a.continue_on_error,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let mut outcomes = db::query::run(&session, &ctx.runs, &id, wrapped, opts).await?;
    if skip_first && !outcomes.is_empty() {
        outcomes.remove(0);
    }
    if skip_last && outcomes.len() > statements.len() {
        outcomes.pop();
    }

    let fmt = out::pick(a.format);
    let many = statements.len() > 1;
    let mut failed = false;
    for (i, o) in outcomes.iter().enumerate() {
        if many && fmt == Fmt::Table {
            eprintln!("{}", term::dim(&format!("-- {} ", one_line(&o.sql, 100))));
        }
        if let Some(e) = &o.error {
            failed = true;
            eprintln!("{} {e}", term::red(&if many { format!("Error in statement {}:", i + 1) } else { "Error:".into() }));
            continue;
        }
        match &o.result {
            Some(rs) => {
                // MongoDB documents print as documents unless a format was asked for.
                let fmt = if a.format.is_none() && rs.has_documents() { out::pick_documents() } else { fmt };
                out::print(rs, fmt, rdialect, table_of(rs), a.wide)?;
                let n = rs.visible_len();
                let more = if rs.truncated { format!(" (first {n} only: --max-rows N or --all for more)") } else { String::new() };
                let unit = if rs.has_documents() { "document" } else { "row" };
                eprintln!("{}", term::dim(&format!("{n} {unit}{} · {}{more}", if n == 1 { "" } else { "s" }, ms(o.elapsed_ms))));
            }
            None => {
                let n = o.affected_rows.unwrap_or(0);
                let id = o.last_insert_id.filter(|&x| x > 0).map(|x| format!(" · last insert id {x}")).unwrap_or_default();
                eprintln!("{}", term::dim(&format!("{n} row{} affected · {}{id}", if n == 1 { "" } else { "s" }, ms(o.elapsed_ms))));
            }
        }
    }
    Ok(if failed { ExitCode::FAILURE } else { ok() })
}

fn table_of(rs: &ResultSet) -> Option<&str> {
    rs.columns.first().and_then(|c| c.table.as_deref())
}

fn one_line(s: &str, max: usize) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { format!("{}…", flat.chars().take(max).collect::<String>()) } else { flat }
}

fn ms(v: f64) -> String {
    if v < 1.0 { format!("{:.0} µs", v * 1000.0) } else if v < 1000.0 { format!("{v:.1} ms") } else { format!("{:.2} s", v / 1000.0) }
}

// ---------- export ----------

fn export_format(path: &str, asked: Option<Fmt>) -> Result<results::Format, String> {
    let fmt = match asked {
        Some(f) => f,
        None => match std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("csv") => Fmt::Csv,
            Some("tsv" | "txt") => Fmt::Tsv,
            Some("json") => Fmt::Json,
            Some("md" | "markdown") => Fmt::Md,
            Some("sql") => Fmt::Sql,
            Some("xlsx") => Fmt::Xlsx,
            _ => return Err(format!("Can't tell the format from \"{path}\": use .csv, .tsv, .json, .xlsx, .md or .sql, or pass --format.")),
        },
    };
    Ok(match fmt {
        Fmt::Csv => results::Format::CsvHeader,
        Fmt::Tsv | Fmt::Table => results::Format::TsvHeader,
        Fmt::Json | Fmt::Docs => results::Format::Json,
        Fmt::Md => results::Format::Markdown,
        Fmt::Sql => results::Format::Insert,
        Fmt::Xlsx => results::Format::Xlsx,
    })
}

pub async fn export(ctx: &Ctx, a: ExportArgs) -> Result<ExitCode, String> {
    let c = ctx.find(&a.connection)?;
    let path = std::path::Path::new(&a.output);
    let format = export_format(&a.output, a.format)?;
    let session = ctx.open(&c).await?;
    let db_name = database(&c, a.database.as_deref());
    let (sql, table) = match (&a.query, &a.table) {
        (Some(q), _) => (q.clone(), None),
        (None, Some(t)) => (export::table_query(&session, db_name.as_deref(), t, a.limit), Some(t.as_str())),
        (None, None) => return Err("Give a table (or collection), or --query.".into()),
    };
    let started = std::time::Instant::now();
    let n = export::query_to_file(&session, &ctx.runs, db_name.as_deref(), sql, table, path, format, a.limit.unwrap_or(50_000_000)).await?;
    println!("Exported {n} row{} to {} · {}", if n == 1 { "" } else { "s" }, a.output, ms(started.elapsed().as_secs_f64() * 1000.0));
    Ok(ok())
}

pub async fn dump(ctx: &Ctx, a: DumpArgs) -> Result<ExitCode, String> {
    let c = ctx.find(&a.connection)?;
    let session = ctx.open(&c).await?;
    let db_name = need_database(&c, &session, a.database.as_deref()).await?;
    let mut path = std::path::PathBuf::from(&a.output);
    if c.engine != Engine::Mongodb && path.extension().is_none() {
        path.set_extension("sql");
    }
    let opts = export::DumpOptions { tables: Some(a.table).filter(|t| !t.is_empty()), data: !a.no_data };
    let started = std::time::Instant::now();
    let stats = export::dump(&session, &ctx.runs, &db_name, &path, &opts).await?;
    println!("Exported {db_name} ({}) to {} · {}", stats.describe(), stats.path.display(), ms(started.elapsed().as_secs_f64() * 1000.0));
    Ok(ok())
}

// ---------- app, settings, version ----------

/// The app's exe next to `tb` (both are installed in the same folder).
fn app_exe() -> Result<std::path::PathBuf, String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = me.with_file_name("tableboat.exe");
    if exe.is_file() { Ok(exe) } else { Err(format!("Tableboat wasn't found next to tb ({}).", exe.display())) }
}

/// Start the app (on a connection); a running app opens it in a new window.
pub fn launch_app(connection_id: Option<&str>) -> Result<(), String> {
    let mut cmd = std::process::Command::new(app_exe()?);
    if let Some(id) = connection_id {
        cmd.args(["--connect", id]);
    }
    cmd.spawn().map(|_| ()).map_err(|e| format!("Could not start Tableboat: {e}"))
}

pub fn open(ctx: &Ctx, query: Option<&str>) -> Result<ExitCode, String> {
    let c = query.map(|q| ctx.find(q)).transpose()?;
    launch_app(c.as_ref().map(|c| c.id.as_str()))?;
    if let Some(c) = c {
        println!("Opening Tableboat on \"{}\"", c.display_name());
    }
    Ok(ok())
}

pub fn settings(cmd: Option<SettingsCmd>) -> Result<ExitCode, String> {
    let path = prefs::path()?;
    let current = prefs::load(&path);
    let value = serde_json::to_value(&current).map_err(|e| e.to_string())?;
    let show = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => format!("\"{s}\""),
        other => other.to_string(),
    };
    match cmd.unwrap_or(SettingsCmd::List) {
        SettingsCmd::List => {
            let obj = value.as_object().ok_or("settings")?;
            let mut keys: Vec<&String> = obj.keys().collect();
            keys.sort();
            for k in keys {
                println!("{:<22} {}", k, show(&obj[k]));
            }
        }
        SettingsCmd::Get { key } => {
            let v = value.get(&key).ok_or_else(|| format!("Unknown setting \"{key}\" (see `tb settings`)."))?;
            println!("{}", show(v));
        }
        SettingsCmd::Set { key, value: raw } => {
            // JSON when it parses (true, 300, ["table"]), else the text itself.
            let v = serde_json::from_str::<serde_json::Value>(&raw).unwrap_or(serde_json::Value::String(raw));
            let (next, _) = current.checked_change(&serde_json::json!({ key.clone(): v }))?;
            let now = serde_json::to_value(&next).map_err(|e| e.to_string())?;
            prefs::save(&path, &next)?;
            println!("{key} = {}", show(&now[&key]));
            eprintln!("{}", term::dim("An open Tableboat window picks this up when it's next focused."));
        }
        SettingsCmd::Path => println!("{}", path.display()),
    }
    Ok(ok())
}

pub fn version() -> Result<ExitCode, String> {
    println!("tb {} ({})", env!("CARGO_PKG_VERSION"), option_env!("TABLEBOAT_COMMIT").unwrap_or("unknown build"));
    let claude_path = claude::find_claude().map(|p| p.display().to_string()).unwrap_or_else(|| "not found".into());
    for (k, v) in [
        ("app", app_exe().map(|p| p.display().to_string()).unwrap_or_else(|e| e)),
        ("data", paths::data_dir()?.display().to_string()),
        ("claude projects", claude::sessions_base().display().to_string()),
        ("claude code", claude_path),
    ] {
        println!("{:<16} {v}", term::dim(k));
    }
    Ok(ok())
}
