# `tb`: Tableboat on the command line

`tb` is a small console program installed next to the app (`%LOCALAPPDATA%\Tableboat\tb.exe`); the
installer adds that folder to your user PATH, so it works in any **new** terminal. It uses the same
saved connections, passwords (Windows Credential Manager), settings and Claude projects as the app,
and works whether or not the app is open.

A connection is named by its name, any unique part of it, or its id: `tb tables "local db"`,
`tb tables mongo`.

Output is a table in a terminal and tab-separated when piped (`tb q … > file.tsv`); choose with
`-f table|docs|csv|tsv|json|md|sql`. MongoDB results print as documents (mongosh syntax) in a
terminal and as JSON (Extended JSON, types kept) when piped; `-f table` gives the grid. Row counts and timings go to stderr, so piped output stays clean.
Exit code 1 on any error.

## Connections

| Command | |
|---|---|
| `tb connections` (`tb ls`) | list saved connections |
| `tb conn show <c>` | settings, URL, Claude project folder |
| `tb conn add <url> [--name N] [--color #hex] [--tag T] [--ask-password] [--test]` | save from a URL: `mysql://`, `mariadb://`, `sqlite:///C:/path/db.sqlite`, `mongodb://`, `mongodb+srv://`, `+ssh` variants |
| `tb conn edit <c> [--name] [--color] [--tag] [--host] [--port] [--user] [--database] [--file] [--ssl-mode] [--password] [--ask-password]` | change a connection (`--password` asks for a new one, hidden) |
| `tb conn rm <c> [--yes]` | delete it and its saved passwords |
| `tb conn test <c or url>` | connect and show the server version |
| `tb conn url <c> [--with-passwords]` | the connection as a URL |
| `tb conn dup <c>` | copy it |
| `tb conn export <file> [--with-passwords]` / `tb conn import <file>` | one URL per line |

## Browse and query

| Command | |
|---|---|
| `tb databases <c>` (`dbs`) | databases / schemas |
| `tb tables <c> [-d db] [--kind table\|view\|function\|procedure\|trigger] [--filter text]` | objects (MongoDB: collections) |
| `tb describe <c> <table> [-d db] [--ddl]` (`desc`) | columns and keys; `--ddl` adds the CREATE statement |
| `tb query <c> "<sql>" [-d db]` (`q`) | run SQL, or mongosh for MongoDB; several statements are fine |
| `tb query <c> -i script.sql` / `… \| tb query <c>` | from a file or a pipe (DELIMITER blocks and procedure bodies are handled) |

`tb query` options:

- `--max-rows N`: most rows kept per result. The default is the app's setting; `--all` removes the limit.
- `--read-only`: the database itself refuses anything that writes.
- `--continue-on-error`: keep going after a statement fails.
- `--wide`: don't shorten long values in table output.
- `--yes`: run changes on a connection tagged **production** without asking. Without it, `tb`
  asks in a terminal and refuses in a script.

## Export and the app

| Command | |
|---|---|
| `tb export <c> <table> -o file.(csv\|tsv\|json\|xlsx\|md\|sql) [-d db] [--limit N]` | a table or collection to a file |
| `tb export <c> --query "<sql>" -o file.xlsx` | a query's result to a file |
| `tb dump <c> -o file.sql [-d db] [-t table,…] [--no-data]` | a whole database as a SQL script (DROP, CREATE, INSERTs, views; MySQL also routines and triggers, SQLite indexes). Restore with `tb query <c> -i file.sql`. MongoDB: `-o` is a folder, one file per collection in canonical Extended JSON (`mongoimport` restores it exactly) |
| `tb open [<c>]` | open the app (on that connection; a running app opens it in a new window) |
| `tb settings` / `get <key>` / `set <key> <value>` / `path` | the app's settings (an open app picks up changes when its window is next focused) |
| `tb version` | version, build, folders, Claude Code location |

## Ask Claude

```
tb ask -c "local db" "which tables reference users?"
tb ask -c logs-db --with "local db" "compare yesterday's errors with new signups"
tb ask --continue -c "local db" "and for last week?"
git diff | tb ask -c "local db" "will this migration lock big tables?"
```

Claude can also work the app, with or without `-c`:

```
tb ask "change the theme to dark"
tb ask "add a connection: mysql on 10.0.0.5, user app, password …, database shop, tag it staging"
tb ask -c "local db" "export the users table to users.xlsx"
tb ask "export the whole inventory database from local db to inventory.sql"
tb ask "export my connections to conns.txt"
```

Claude Code (`claude.exe`) runs in the connection's project folder (`~\.tableboat\sessions\<connection>`,
shared with the app's Claude pane, including its `CLAUDE.md` notes), with Tableboat's database tools:

- **Read freely:** schema, context, read-only queries (the database enforces read-only), the saved
  connections (never their passwords), the settings; testing a connection and opening one in the app.
- **Ask first:** anything that changes something or writes a file (SQL that writes, settings, saving
  or deleting a connection, exports) shows what it will do and asks `Allow? [y]es / [N]o / [a]lways`
  in the terminal. Files given without a folder go in the current folder.
- **Scripts:** `--no-input` denies everything that would ask.

Options:

- `--with <c>` (repeat it) lets Claude also read other connections.
- `--model sonnet|opus|haiku` picks the model; the default is the app's setting.
- `--continue` picks up the last chat for that connection.

## Environment

- `TABLEBOAT_PASSWORD`: the password for a connection that asks every time, when there's no console.
- `TABLEBOAT_DATA_DIR`: use another data folder (tests, a portable copy).
- `NO_COLOR` or `--no-color`: no colours.
