//! `tb`'s commands and options (clap also builds the help from these).

use clap::{Args, Parser, Subcommand, ValueEnum};

const EXAMPLES: &str = "\
Examples:
  tb connections                                   list saved connections
  tb tables \"local db\"                             tables in the connection's database
  tb describe \"local db\" users --ddl                columns, keys and CREATE TABLE
  tb query \"local db\" \"select id, name from users limit 5\"
  tb query logs-db \"db.errors.find({}).limit(3)\" -f json
  tb query \"local db\" -i report.sql -f csv > report.csv
  tb export \"local db\" orders -o orders.xlsx
  tb dump \"local db\" -o inventory.sql                whole database as a SQL script
  tb ask -c \"local db\" \"which tables reference users?\"
  tb ask -c logs-db --with \"local db\" \"compare yesterday's errors with new signups\"

A connection is given by its name (any unique part of it works) or its id.";

#[derive(Parser)]
#[command(
    name = "tb",
    version,
    about = "Tableboat on the command line: connections, queries, exports and Claude.",
    after_help = EXAMPLES,
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
    /// Never use colours
    #[arg(long, global = true)]
    pub no_color: bool,
}

#[derive(Subcommand)]
pub enum Command {
    /// List saved connections (same as `tb conn list`)
    #[command(visible_alias = "ls")]
    Connections(ListArgs),
    /// Add, edit, remove, test, import and export saved connections
    #[command(subcommand, name = "conn", visible_alias = "connection")]
    Conn(ConnCmd),
    /// List the databases (schemas) on a connection
    #[command(visible_alias = "dbs")]
    Databases {
        connection: String,
        #[arg(short, long, value_enum)]
        format: Option<Fmt>,
    },
    /// List tables, views, functions, procedures and triggers (MongoDB: collections)
    Tables(TablesArgs),
    /// Show a table's columns and keys (and its CREATE statement with --ddl)
    #[command(visible_alias = "desc")]
    Describe(DescribeArgs),
    /// Run SQL (mongosh for MongoDB) and print the results
    #[command(visible_alias = "q")]
    Query(QueryArgs),
    /// Export a table or a query to a file: CSV, JSON, Excel, Markdown, SQL or TSV
    Export(ExportArgs),
    /// Export a whole database: a SQL script (MySQL, SQLite) or a folder of JSON files (MongoDB)
    Dump(DumpArgs),
    /// Ask Claude Code about your data (the same tools as the app's Claude pane)
    Ask(AskArgs),
    /// Open the Tableboat app, optionally on a connection
    Open { connection: Option<String> },
    /// Show or change settings (shared with the app)
    Settings {
        #[command(subcommand)]
        cmd: Option<SettingsCmd>,
    },
    /// Version, build and where Tableboat keeps its files
    Version,
}

#[derive(Args)]
pub struct ListArgs {
    #[arg(short, long, value_enum)]
    pub format: Option<Fmt>,
}

#[derive(Subcommand)]
pub enum ConnCmd {
    /// List saved connections
    #[command(visible_alias = "ls")]
    List(ListArgs),
    /// Show one connection's settings
    Show { connection: String },
    /// Save a connection from a URL, e.g. mysql://user:pass@host:3306/db
    Add(AddArgs),
    /// Change a saved connection
    Edit(EditArgs),
    /// Delete a saved connection (and its saved passwords)
    #[command(visible_alias = "remove")]
    Rm {
        connection: String,
        /// Don't ask for confirmation
        #[arg(short, long)]
        yes: bool,
    },
    /// Connect and report the server version (a saved connection or a URL)
    Test { target: String },
    /// Print a connection as a URL
    Url {
        connection: String,
        /// Include saved passwords
        #[arg(long)]
        with_passwords: bool,
    },
    /// Copy a connection (with its saved passwords)
    #[command(visible_alias = "dup")]
    Duplicate { connection: String },
    /// Write all connections to a file, one URL per line
    Export {
        file: String,
        /// Include saved passwords (keep the file private)
        #[arg(long)]
        with_passwords: bool,
    },
    /// Add the connections from a file of URLs (one per line)
    Import { file: String },
}

#[derive(Args)]
pub struct AddArgs {
    /// mysql://, mariadb://, sqlite:///path, mongodb://, mongodb+srv:// (+ssh variants)
    pub url: String,
    /// Nickname shown everywhere
    #[arg(long)]
    pub name: Option<String>,
    /// Colour, e.g. #16a34a
    #[arg(long)]
    pub color: Option<String>,
    /// local, development, testing, staging, production or your own label
    #[arg(long)]
    pub tag: Option<String>,
    /// Don't save the password: ask every time
    #[arg(long)]
    pub ask_password: bool,
    /// Connect once before saving, to check it works
    #[arg(long)]
    pub test: bool,
}

#[derive(Args)]
pub struct EditArgs {
    pub connection: String,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub color: Option<String>,
    /// A label, or "" to remove it
    #[arg(long)]
    pub tag: Option<String>,
    #[arg(long)]
    pub host: Option<String>,
    #[arg(long)]
    pub port: Option<u16>,
    #[arg(long)]
    pub user: Option<String>,
    /// Default database (schema)
    #[arg(long)]
    pub database: Option<String>,
    /// SQLite database file
    #[arg(long)]
    pub file: Option<String>,
    /// disabled, preferred, required, verify-ca or verify-identity
    #[arg(long)]
    pub ssl_mode: Option<String>,
    /// Type a new password (hidden) and save it
    #[arg(long, conflicts_with = "ask_password")]
    pub password: bool,
    /// Stop saving the password: ask every time
    #[arg(long)]
    pub ask_password: bool,
}

#[derive(Args)]
pub struct TablesArgs {
    pub connection: String,
    /// Database (schema); default: the connection's
    #[arg(short, long)]
    pub database: Option<String>,
    /// Only this kind
    #[arg(short, long, value_enum)]
    pub kind: Option<Kind>,
    /// Only names containing this text
    #[arg(long)]
    pub filter: Option<String>,
    #[arg(short, long, value_enum)]
    pub format: Option<Fmt>,
}

#[derive(Args)]
pub struct DescribeArgs {
    pub connection: String,
    pub table: String,
    #[arg(short, long)]
    pub database: Option<String>,
    /// Also print the CREATE statement (MongoDB: the collection's options and indexes)
    #[arg(long)]
    pub ddl: bool,
    #[arg(short, long, value_enum)]
    pub format: Option<Fmt>,
}

#[derive(Args)]
pub struct QueryArgs {
    pub connection: String,
    /// The SQL (several statements are fine); or use --file, or pipe it in
    pub sql: Vec<String>,
    #[arg(short, long)]
    pub database: Option<String>,
    /// Read the statements from a file
    #[arg(short = 'i', long = "file")]
    pub file: Option<String>,
    /// Output format (default: table in a terminal, tsv when piped)
    #[arg(short, long, value_enum)]
    pub format: Option<Fmt>,
    /// Most rows kept per result (default: the app's setting)
    #[arg(long)]
    pub max_rows: Option<usize>,
    /// No row limit
    #[arg(long, conflicts_with = "max_rows")]
    pub all: bool,
    /// Refuse anything that could change data (enforced by the database)
    #[arg(long)]
    pub read_only: bool,
    /// Allow changes on a connection tagged "production" without asking
    #[arg(short, long)]
    pub yes: bool,
    /// Keep going after a statement fails
    #[arg(long)]
    pub continue_on_error: bool,
    /// Show long values in full in table output
    #[arg(long)]
    pub wide: bool,
}

#[derive(Args)]
pub struct ExportArgs {
    pub connection: String,
    /// The table or collection (or use --query)
    pub table: Option<String>,
    /// Export the result of this query instead of a table
    #[arg(short, long, conflicts_with = "table")]
    pub query: Option<String>,
    /// The file to write; the format follows its extension unless --format is given
    #[arg(short, long)]
    pub output: String,
    #[arg(short, long)]
    pub database: Option<String>,
    #[arg(short, long, value_enum)]
    pub format: Option<Fmt>,
    /// Most rows to export (default: all)
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Args)]
pub struct DumpArgs {
    pub connection: String,
    /// The .sql file to write (MongoDB: a folder, one JSON file per collection)
    #[arg(short, long)]
    pub output: String,
    #[arg(short, long)]
    pub database: Option<String>,
    /// Only these tables or collections (repeat it, or separate with commas)
    #[arg(short, long, value_delimiter = ',')]
    pub table: Vec<String>,
    /// Structure only, no rows
    #[arg(long)]
    pub no_data: bool,
}

#[derive(Args)]
pub struct AskArgs {
    /// Your question (or pipe it in)
    pub question: Vec<String>,
    /// The connection Claude works on
    #[arg(short, long)]
    pub connection: Option<String>,
    /// Database (schema); default: the connection's
    #[arg(short, long)]
    pub database: Option<String>,
    /// Another connection Claude may read too (repeat for more)
    #[arg(long = "with", value_name = "CONNECTION")]
    pub with: Vec<String>,
    /// Model, e.g. sonnet, opus, haiku (default: the app's setting)
    #[arg(long)]
    pub model: Option<String>,
    /// Continue the last chat for this connection
    #[arg(long = "continue")]
    pub resume: bool,
    /// Never ask: deny anything that needs approval (for scripts)
    #[arg(long)]
    pub no_input: bool,
}

#[derive(Subcommand)]
pub enum SettingsCmd {
    /// All settings and their values
    List,
    /// One setting's value
    Get { key: String },
    /// Change a setting, e.g. `tb settings set theme dark`
    Set { key: String, value: String },
    /// Where settings.json is
    Path,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Fmt {
    Table,
    /// MongoDB documents in mongosh syntax (the default for MongoDB in a terminal)
    Docs,
    Csv,
    Tsv,
    Json,
    #[value(alias = "markdown")]
    Md,
    /// INSERT statements
    Sql,
    /// Excel (export only)
    Xlsx,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    Table,
    View,
    Function,
    Procedure,
    Trigger,
}
