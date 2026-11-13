# Tableboat – build plan

What to build is in [`SCOPE.md`](SCOPE.md). This file is how and in what order.
It was planned from background research (other database clients' features, changelogs and public issues, libraries
and benchmarks, visual trends). Those notes are kept privately and are not part of this repository.

---

## 1. Decisions

| Decision | Choice | Why |
|---|---|---|
| Starting point | **Fresh Tauri 2 project.** Borrow code (with licence headers) from Tabularis and DBX (both Apache-2.0) where it helps | Tabularis uses Monaco, a DOM grid and an old SSH crate; DBX is Vue with 100+ engines. Forking would mean replacing what matters |
| Runtime | One Rust executable + the WebView2 already in Windows. **No Node.js at runtime** | Your RAM/CPU limit. Node and npm only bundle the UI at build time |
| AI pane | Drive your installed `claude.exe` from Rust in stream-JSON mode (no ACP adapter, which is Node) | Same protocol the adapter uses, minus a Node process |

## 2. Stack

| Part | Library | Notes |
|---|---|---|
| App shell | Tauri 2.12 | Plugins: dialog, window-state, single-instance |
| PostgreSQL | tokio-postgres 0.7 + rustls | Built-in query cancel; all values read as text so every type round-trips |
| MySQL / MariaDB | mysql_async 0.37 | Text protocol, so `0000-00-00` dates survive; cancel with `KILL QUERY` on a second connection |
| SQLite | rusqlite 0.41 (bundled) | Cancel with `interrupt()` |
| MongoDB | mongodb 3.10 | Shell syntax (`db.x.find(...)`) parsed in the UI with acorn into commands; a documented subset, not a full shell |
| SSH tunnel | russh 0.65 + ssh2-config 0.9 | OpenSSH/PEM/ed25519/ECDSA keys, passphrases, Pageant, Windows OpenSSH agent, 1Password agent, `~/.ssh/config`. PuTTY `.ppk` needs our own reader (later) |
| Passwords | keyring-core 1.0 + Windows Credential Manager store | One entry per secret: `Tableboat/<connection-id>/<field>` |
| UI framework | React 19 | Required by the grid; only ~1 MB more than Solid in our benchmark |
| Results grid | Glide Data Grid 6 (canvas) | 24 ms/frame on 2,000 × 1,000 cells at 4× CPU throttle vs 70–153 ms for others. Wrapped in our own adapter, because its last release was Feb 2024 |
| SQL editor | CodeMirror 6 + a VS Code Dark+/Light+ theme | ~4 MB vs Monaco's 10–12 MB. Multiple cursors, find/replace, folding built in |
| Statement splitting | sqlparser 0.63 tokenizer (Rust) + our own `DELIMITER` / `BEGIN…END` / `$$` handling | Same splitter decides "statement under cursor" and "run all" |
| SQL formatting | sql-formatter 15.9 (runs in the WebView) | The only maintained formatter with PostgreSQL, MySQL and SQLite dialects |
| Fuzzy search | frizbee 0.13 (Rust) | Ctrl+P, sidebar filter, palette |
| Palettes, menus, dialogs | cmdk + Radix primitives | Unstyled; we style them |
| Exports | csv 1.4, rust_xlsxwriter 0.102, our own Markdown / JSON / INSERT writers | All in Rust, streamed to disk |
| MCP server (for Claude) | rmcp 3.9, in-process, `127.0.0.1` with a random bearer token | Database tools + the permission-prompt tool |

## 3. Architecture

```
Tableboat.exe
├─ Rust core (src-tauri/src)
│  ├─ conn/      saved connections, URL import/export, secrets, SSH tunnels
│  ├─ db/        one Driver trait; postgres.rs · mysql.rs · sqlite.rs · mongo.rs
│  ├─ query/     splitter, runner (cancel, timing, messages), result store, pending edits → SQL
│  ├─ meta/      object lists, DDL, schema cache (for sidebar, Ctrl+P, autocomplete)
│  ├─ export/    CSV · JSON · Markdown · INSERT · XLSX
│  ├─ search/    fuzzy index over objects and commands
│  ├─ ai/        claude.exe bridge (stream-JSON) + MCP server
│  └─ settings/  settings.json, window and tab state
└─ WebView UI (src)  React
   ├─ shell/     title bar, status pill, panes, tabs, themes
   ├─ sidebar/   DB/schema switchers, object tree, filter-on-type
   ├─ editor/    CodeMirror, run/cancel, autocomplete
   ├─ results/   grid adapter, result tabs, pending edits, copy/export
   ├─ palette/   Ctrl+P quick open, Ctrl+Shift+P commands
   ├─ settings/  settings page
   └─ ai/        chat pane
```

**Keeping memory low**

- Result rows live in Rust in a compact buffer. The grid asks for only the rows on screen (blocks of ~200), and the UI keeps a small cache of blocks. A 1-million-row result never sits in the WebView.
- Table tabs load one page (300 rows by default) with `LIMIT/OFFSET`. Query results stream into Rust up to a cap (setting, default 50,000) with "Fetch more".
- Exports re-run the query and stream straight to the file.
- Closing a tab frees its result buffer. Closing a connection closes its pool and tunnel.
- Nothing polls. The only timer is an optional keep-alive ping (every 60 s, setting).
- `claude.exe` starts when the AI pane opens and stops when it closes.
- WebView2's low-memory mode is switched on when the window is minimised.

**Where things are stored**

- `%APPDATA%\Tableboat\connections.json`: connections without passwords
- `%APPDATA%\Tableboat\settings.json`: settings (also editable by hand)
- `%APPDATA%\Tableboat\state\`: open tabs, editor text, column widths
- Windows Credential Manager: passwords and SSH key passphrases
- `~\.tableboat\sessions\<connection>\`: Claude Code's working directory, one project per connection (its own chats, CLAUDE.md notes and memory); `general\` when not connected

## 4. Visual direction

Familiar TablePlus layout, quieter 2026 Windows skin. Details in the (private) visual research notes.

- **Title bar (40 px)** with the sidebar/AI toggles and a centred **connection pill**: tag chip (`PROD`, `LOCAL`), driver and version, nickname, `db.schema`. The connection colour tints the pill and a 2 px line under the title bar, never the whole bar. Pill text colour picked automatically for contrast. Windows Snap Layouts work.
- **Tabs (30 px)**, content-width; the active tab joins the editor below it; × on hover.
- **Sidebar** a shade darker than the content, monospace object names at 12 px, neutral (not blue) selection, matched letters highlighted while filtering.
- **Grid**: 24 px rows (Compact 22, Comfortable 28); Cascadia Mono / Consolas 12–13 px with aligned digits, numbers right-aligned; header shows name plus muted type and key icons; `NULL` muted italic but readable; active cell is a 2 px accent ring; selected rows a soft tint.
- **Pending edits**: amber edited cells, green new rows, red struck-through deleted rows; a **Save N** button with Discard and Preview next to it.
- **Palettes**: Raycast-style, 640 px wide, sections, each row with icon · name · muted schema · kind, and a footer showing the keys.
- **Themes**: Light, Dark and System, built from a few tokens (base, accent, contrast, density). Dark uses neutral Windows greys (`#202020` / `#1c1c1c` / `#282828`), accent blue `#4c8dff`.
- **Shape and type**: corners 4 px (controls), 6 px (pill), 8 px (popups); Segoe UI Variable for the UI; our own 16 px outline icons.
- **Cheap on purpose**: solid surfaces (no blur or Acrylic), no spinners or shimmer, a 2 px progress bar only while a query runs, short open/close motion only.
- **Crisp text**: no transparency or transforms on text layers; tested at 100/125/150 % and across mixed-DPI monitors.

## 5. Milestones

Each milestone ends with: a build you run and try, measured RAM (private working set of the app + WebView2) and idle CPU, and the relevant `SCOPE.md` items ticked off by you.

| # | Milestone | What you can do at the end |
|---|---|---|
| **M0** | **Shell** – Tauri + React project, title bar with Snap Layouts, resizable left / centre / right panes, themes and density, `settings.json`, empty Ctrl+Shift+P | Open the app, switch theme and density, resize panes. Baseline RAM measured |
| **M1** | **Connections** – start screen, form per engine, nickname + colour + tag, Credential Manager, test, URL import / copy as URL, export / import file, SSL, SSH tunnel. **MySQL / MariaDB** (priority) and SQLite drivers | Save and connect to your MySQL / MariaDB databases, including over SSH |
| **M2** | **Sidebar and Ctrl+P** – database switcher, tables / views / functions / procedures / triggers, filter-on-type with fuzzy matching, show DDL, Ctrl+P across all databases | Find and open any object by keyboard |
| **M3** | **SQL editor and results** – VS Code-coloured editor, splitter (incl. `DELIMITER`), run current / run all, cancel (`KILL QUERY`), autocomplete, format; grid, result tabs, messages, quick search, copy-as and export in every format | Daily querying on MySQL / MariaDB |
| **M4** | **Editing** – edit / NULL / add / duplicate / delete rows, pending highlights, Save N, Discard, Preview, unsaved-changes prompts | Edit data safely with one Save |
| **M5** | **PostgreSQL and MongoDB** – drivers, schema switcher, object lists, editor modes, editing (Mongo by `_id`) | All four engines working |
| **M6** | **Palette and settings** – every command in Ctrl+Shift+P with shortcuts, quick toggles, full settings page with search, tabs and editor text restored after restart | Configure everything from the keyboard |
| **M7** | **AI pane** – `claude.exe` bridge in `~\.tableboat\sessions\<connection>`, streaming chat, tool calls, Allow / Deny, stop, sessions list and resume, Insert / Run SQL buttons, MCP tools for schema and read-only queries | Ask Claude about your database beside your work |
| **M8** | **Finish** – per-user installer (no admin), edge-case test pass (below), final RAM / CPU report | Install it like any app |

## 6. Edge cases built in from the research

Taken from TablePlus's own bugs (its public issue trackers and changelogs); each becomes a test.

- **Safe writes**: values bound as parameters, never pasted into SQL (typing `888--` must not touch other rows). No `UPDATE`/`DELETE` without a key-based `WHERE`. SQLite rows with `NULL`s deletable (`IS NULL`). MySQL generated columns never written.
- **Pending edits**: no false "changed" marks; Esc cancels an edit; empty string ≠ `NULL`; an edit still being typed is included when you save.
- **Types**: Postgres arrays, enums, JSON/JSONB (no float rounding, no lost trailing zeros), `bytea` as hex; MySQL zero dates, `BIT`, `BINARY(16)` keys, utf8mb4 emoji; big numbers sorted as numbers; booleans exported as `true`/`false`.
- **Locale**: Windows regional settings never leak into data (no comma decimals, no `12/5/2024 6:45 PM` dates); CSV is RFC 4180.
- **Editor**: "statement under cursor" right with strings, comments, `#`, `$$`, `BEGIN…END`, `DELIMITER`; 200+ statements in one run; large `.sql` files don't freeze.
- **Connections**: passwords with spaces and `@ < > :`; `mongodb+srv://` with encoded credentials; reconnect after sleep with one message, not one per tab; listing databases denied → fall back to the current one; secrets never lost on update or edit.
- **Windows**: crisp at mixed DPI; AltGr keyboard layouts don't trigger shortcuts; clipboard retries when another app holds it; SQLite on `\\wsl$` opened read-only with a warning.

## 7. Risks

| Risk | Plan |
|---|---|
| 120 MB idle target is tight (WebView2 alone measures ~90 MB) | Measure from M0; lazy-load the editor, grid and AI pane; low-memory mode when minimised. Report real numbers either way |
| Glide Data Grid has had no release since Feb 2024 | Pinned, behind our adapter; RevoGrid is the fallback |
| `claude.exe` stream-JSON and permission-tool formats can change between versions | Check against 2.1.319 in M7; version check on start with a clear message |
| Local MCP port is reachable by other programs on the PC | Random bearer token per launch, Origin check, bound to `127.0.0.1` only |
| MongoDB shell syntax is a subset | Document what's supported; a JSON "command" mode as fallback |
| Query cancel differs per engine | Implemented per driver and tested in M3/M5 |
