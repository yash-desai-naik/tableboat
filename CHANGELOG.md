# Changelog

All notable changes to Tableboat are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-11-14

### Added

- **Claude can do what you can do in the app.** In the Claude pane it can now:
  - see what's on screen: the connection, database, open tabs, unsaved edits and results;
  - run any command from the command palette: theme, zoom, tabs, run or format SQL, save or
    discard table edits, disconnect, and more (new palette commands are included automatically);
  - open tables, views and routines in tabs, put SQL into the editor (and run it), and switch
    the database;
  - list databases and objects, show any object's definition, and duplicate connections;
  - check for updates and install one.
- `execute_sql` from Claude runs whole scripts and takes a database.
- Anything that can change data or lose work still asks you first.

### Fixed

- Builds made by GitHub no longer say "with uncommitted changes" in the About box.

## [0.1.0] - 2026-11-14

The first public release: a small, fast database client for Windows, with a command line and
Claude Code built in.

### Highlights

- **MySQL, MariaDB, SQLite and MongoDB** in one light app: about 140 MB of memory with a
  connection open, no CPU use while idle, no Electron and no Node.js at runtime.
- **MongoDB documents as documents:** nesting and every type (`ObjectId`, `ISODate`,
  `NumberLong`, decimals, UUIDs) shown, edited, copied and exported without loss.
- **`tb` command line:** connections, queries, exports, database dumps and Claude, from any
  terminal.
- **Claude Code pane:** ask about your data next to the editor. Claude reads freely, and asks
  before anything that changes data, settings or files.
- **Updates built in:** Tableboat tells you when a new version is out, shows what's new, and
  installs it with one click (signed, verified, then restarts).

### Added

- **Connections**
  - MySQL, MariaDB, SQLite (open or create a file) and MongoDB, including `mongodb+srv://`.
  - SSH tunnels (password, key or agent) with trust-on-first-use host keys, and SSL modes.
  - Passwords stored in Windows Credential Manager, or asked every time.
  - Add a connection by pasting its URL; export and import connection files; colours and tags.
  - Several windows, each on its own connection; the last connection and its tabs come back at
    start-up.
- **Browsing**
  - A searchable sidebar with databases, tables, views, functions, procedures and triggers.
  - Table data a page at a time, sorted on the server; table definitions (`CREATE` statements).
- **SQL editor**
  - Several statements per tab; run the current one or all of them, and cancel.
  - Autocomplete from the schema, a formatter, and `DELIMITER` blocks for procedures.
  - MongoDB queries in mongosh syntax, e.g. `db.users.find({ age: { $gt: 30 } })`.
- **Results**
  - A fast grid that only draws the rows on screen; quick search and sort.
  - Copy and export as CSV, Excel, JSON, Markdown or SQL `INSERT` statements.
  - Edit cells, add, duplicate and delete rows, preview the SQL, and save in one transaction.
  - A JSON view for SQL results; copy one row or all rows as JSON.
- **MongoDB**
  - A Documents view (the default) and a Table view.
  - Edit, duplicate, add and delete whole documents in mongosh syntax; the `_id` is protected.
  - JSON exports in Extended JSON; database dumps in canonical Extended JSON that `mongoimport`
    restores exactly.
- **Claude Code pane** (needs [Claude Code](https://claude.com/claude-code), signed in)
  - One Claude project per connection, with its own notes (`CLAUDE.md`) and chat history.
  - Read-only queries that the database itself enforces; SQL that writes always asks first.
  - Add other connections to a chat with `@`; attach files and images; voice typing.
  - App tools: create, edit, test and delete connections; change settings; export a table or
    query; dump a whole database. Each change asks first and never shows a password.
- **`tb` command line**, installed next to the app and added to your PATH
  - `tb connections`, `tb conn add|edit|rm|test|url|export|import`.
  - `tb databases`, `tb tables`, `tb describe`, `tb query` (also from files and pipes).
  - `tb export` (CSV, Excel, JSON, Markdown, SQL), `tb dump` (whole databases).
  - `tb ask` (Claude in the terminal), `tb open`, `tb settings`, `tb version`.
  - Safety: connections tagged `production` refuse writes unless you confirm; `--read-only`.
- **Updates**
  - A check at start-up and once a day (Settings > Start-up), and "Check for updates" in the
    About box and the command palette.
  - Install and restart, What's new, Later, or Skip this version.
  - Updates are signed; Tableboat verifies the signature before installing, and keeps your tabs.
- Light and dark themes, three densities, zoom, a command palette, and an About box with the
  version, build and data folders.

[Unreleased]: https://github.com/yash-desai-naik/tableboat/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/yash-desai-naik/tableboat/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yash-desai-naik/tableboat/releases/tag/v0.1.0
