# Tableboat – v1 scope

A deliberately small database client. It keeps the parts of TablePlus and VS Code you actually use, and nothing else.
Built with Tauri 2 (Rust backend, WebView2 UI). Name, icons and artwork are our own.

The background research (kept privately, not in this repository) is reference for later; it is **not** the v1 target.

Status: `[ ]` not started · `[~]` in progress · `[x]` done and checked by you.

---

## Budget (applies to everything below)

- [ ] Idle with one connection open: under 120 MB total (app + WebView2), not counting the AI pane
- [ ] No CPU use while idle: no polling, no running animations
- [ ] Results are fetched in pages and the grid only draws visible rows
- [ ] The AI pane's process starts only when the pane is opened, and stops when it's closed
- [ ] **No Node.js at runtime.** The shipped app is one Rust executable plus the WebView2 already on Windows. Node is used only on this PC at build time to bundle the UI, and isn't installed with or run by the app
- [ ] RAM and CPU measured and reported at each milestone

## Databases

- [ ] PostgreSQL (and compatibles: Supabase, Neon, CockroachDB)
- [ ] MySQL / MariaDB
- [ ] SQLite (open a file, create a file)
- [ ] MongoDB (collections in the sidebar; queries in shell syntax, e.g. `db.users.find({...})`)

## 1. Connections

- [ ] Saved connection list on the start screen, with search
- [ ] Each connection has a **nickname** and a **colour**; the colour shows on the connection's window/status bar
- [ ] Form per database type: host, port, user, password, database (SQLite: file path)
- [ ] SSL mode, and an SSH tunnel (password or key)
- [ ] Passwords stored in Windows Credential Manager, never in the connections file
- [ ] Test, Save, Connect, Edit, Duplicate, Delete
- [ ] **Import from URL**: paste `postgres://user:pass@host:5432/db?sslmode=require` (also `mysql://`, `sqlite:///path`, `mongodb://`, `mongodb+srv://`) to fill the form
- [ ] **Copy as URL** (with or without the password)
- [ ] **Export / import connections** as a file of URLs (one per line), with passwords left out unless you choose to include them
- [ ] Several connections open at once (separate windows)

## 2. Sidebar (left)

- [ ] Database switcher and schema switcher for the current connection
- [ ] Object tree: tables, views, functions/procedures, triggers (MongoDB: collections)
- [ ] VS Code-style **filter-on-type**: start typing with the tree focused to filter it, with fuzzy matching and the matched letters highlighted
- [ ] A search box at the top with the same filter, plus toggles to show or hide each object type
- [ ] Opening a table or view opens a tab that runs `SELECT * … LIMIT <page size>`
- [ ] Context menu: open, copy name, copy `SELECT`, show definition (DDL)
- [ ] Refresh (`Ctrl+R`)

## 3. SQL editor

- [ ] Tabs, one editor per tab; `Ctrl+T` new, `Ctrl+W` close
- [ ] **VS Code-like syntax highlighting** (VS Code's Dark+/Light+ colours) for each SQL dialect, plus JavaScript for MongoDB
- [ ] Multi-line queries and several queries per tab
- [ ] **Run current statement** (`Ctrl+Enter`): the statement under the cursor, which is highlighted; or the selection, if any
- [ ] **Run all** (`Ctrl+Shift+Enter`): every statement, one result tab each
- [ ] Statement splitting that handles strings, comments, `$$` bodies, `BEGIN … END` and MySQL `DELIMITER`
- [ ] Autocomplete for keywords, tables, columns and aliases
- [ ] Cancel a running query
- [ ] Format SQL; toggle comment (`Ctrl+/`); find and replace; multiple cursors
- [ ] Editor tabs and their text survive a restart

## 4. Results pane

- [ ] Fast, virtualised grid; resizable columns; click a header to sort the loaded rows
- [ ] `NULL` shown distinctly (muted italic); column type shown muted in the header
- [ ] Row count, time taken, and server messages / errors (with an error tab)
- [ ] One tab per result when several statements run
- [ ] Quick search within the results
- [ ] Select cells, rows or columns; `Ctrl+C` copies as tab-separated text (pastes into Excel/Sheets)
- [ ] **Copy as** Markdown table, CSV, CSV with header, JSON, SQL `INSERT`, plain text
- [ ] **Export** to CSV, JSON, Markdown, SQL `INSERT` and Excel (`.xlsx`): the loaded rows, or re-run and stream the full result to a file
- [ ] **Editable grid with pending changes.** Double-click or `Enter` to edit a cell, set `NULL`, add rows, duplicate rows and delete rows. Nothing is written as you type: changed cells, new rows and deleted rows are highlighted and kept pending
- [ ] **One Save button** (`Ctrl+S`) writes every pending change in a single transaction, showing the number of pending changes (e.g. "Save 3"). If any statement fails, the whole save is rolled back and the error is shown next to the row
- [ ] **Discard** throws away all pending changes; **Preview** shows the exact SQL that Save will run
- [ ] Changes are written with bound parameters and a primary-key `WHERE`, never by pasting values into SQL. Results without a usable key are read-only, with a note saying why
- [ ] Editable when the result comes from one table (opened from the sidebar, or a simple `SELECT` on one table); joins and computed columns are read-only
- [ ] Closing a tab or switching table with unsaved changes asks Save / Discard / Cancel

## 5. Quick open – `Ctrl+P`

- [ ] Fuzzy search across every database and schema of the current connection: tables, views, functions, procedures, triggers, schemas, databases
- [ ] Each result shows its type and schema; `^` / `$` anchors
- [ ] `Enter` opens; `Ctrl+Enter` opens in a new tab; `Esc` closes

## 6. Command palette – `Ctrl+Shift+P`

- [ ] Fuzzy list of every command, showing each one's shortcut
- [ ] Quick toggles: theme (light / dark / system), density (compact / default), sidebar, AI pane, word wrap, font size, page size, show/hide object types
- [ ] "Open Settings" opens the full settings page
- [ ] **Settings page**: all preferences, searchable, saved to a JSON file you can also edit by hand

## 7. AI pane (right) – Claude Code over ACP

- [ ] Toggle with a shortcut and from the command palette
- [ ] Runs your installed `claude.exe` directly from Rust in streaming-JSON mode (`-p --input-format stream-json --output-format stream-json`), using your existing Claude Code login. No Node.js: the official ACP adapter is a Node program, so Tableboat speaks to Claude Code the same way the adapter does, without it
- [ ] Permission prompts come back to Tableboat through `--permission-prompt-tool` on Tableboat's own MCP server, and are shown as Allow / Deny buttons
- [ ] **Working directory is `~\.tableboat\sessions\<connection>`**: one Claude Code project per connection, created if missing (changed from the original `D:\tableboat\sessions` so the app works on any PC)
- [ ] Chat with streaming replies, tool-call and permission prompts, and a stop button
- [ ] Sessions listed and resumable
- [ ] "Insert into editor" and "Run" buttons on SQL in replies
- [ ] Tableboat gives Claude the current connection's context (schema, selected table, current query) through a local MCP server; read-only by default, and write queries need your approval
- [x] Claude can also work the app itself (pane and `tb ask`): read and change settings, create, edit, test and delete saved connections, export/import connection files, export a table or query to a file, export a whole database (SQL script; MongoDB: JSON per collection), open a connection in a new window. Reading is free; every change asks first with a plain summary (never showing passwords)

---

## Not in v1 (from the TablePlus research; can be added later)

Structure editor · row filter bar · row detail panel · import CSV/SQL into tables · backup/restore · process list · user management · ER diagram · metrics board · plugins · safe-mode levels.
