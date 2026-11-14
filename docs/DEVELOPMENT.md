# Developing Tableboat

All commands run from the project folder, `D:\projects\tableplus-clone`, in a normal terminal
(Rust's `cargo` is on the user PATH; if a build says `cargo ... program not found`, open a new terminal).

## Run while developing

```powershell
npx tauri dev
```

- The first start compiles the Rust side (a few minutes); later starts are much quicker.
- Changes under `src\` (React, CSS) show in the open window on save.
- Changes under `src-tauri\src\` recompile and restart the app.
- Dev mode uses the same connections, settings and saved passwords as the installed app.

With the browser developer tools (open `http://127.0.0.1:9222` in Edge or Chrome):

```powershell
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"; npx tauri dev
```

## Code layout (Rust, `src-tauri\src\`)

- `lib.rs`: the window-free core `tableboat_core`, shared by both programs. It holds:
  - `conn`: saved connections, URLs, passwords;
  - `db`: engines, queries, results, the statement splitter, exports and dumps (`db/export.rs`);
  - `prefs`: settings;
  - `claude`: claude.exe and per-connection projects;
  - `mcp`: the tool server Claude uses, behind a `Host` trait: database tools in `mcp.rs`, app
    tools (settings, connections, exports) in `mcp_app.rs`; tools that change something ask the
    user through `Host::ask` themselves (`claude::ASKS_ITSELF`);
  - `paths`.
- `main.rs` with `ai`, `commands`, `settings`, `state`: the desktop app (Tauri). `ai/host.rs` plugs
  each window into the tool server.
- `bin/tb/`: the `tb` command line (see [CLI.md](CLI.md)).

```powershell
cd src-tauri
cargo build --bin tb                 # just the command line (fast)
.\target\debug\tb.exe connections
$env:TABLEBOAT_DATA_DIR = "$env:TEMP\tb-test"   # try commands without touching your real connections
```

## Checks

```powershell
npm run typecheck                    # TypeScript
cd src-tauri; cargo test; cd ..      # Rust (live DB tests run only when their URLs are set, see src-tauri\src\db\mod.rs)
node scripts/test-splitter.ts        # SQL statement splitter
node scripts/test-readonly.ts        # read-only query detection (refresh / re-run)
```

## Build the installer

```powershell
npx tauri build
```

- About 6-7 minutes, up to about 1.5 GB of RAM near the end.
- Installer: `src-tauri\target\release\bundle\nsis\Tableboat_<version>_x64-setup.exe`.
  It installs for the current Windows user only (no admin prompt) into `%LOCALAPPDATA%\Tableboat`:
  the app and `tb.exe`, and adds that folder to the user PATH (`src-tauri\windows\hooks.nsh`;
  uninstalling takes it off again).
- Running a newer setup upgrades in place. To change the version Windows shows, edit `"version"`
  in `src-tauri\tauri.conf.json` first.
- The app alone, without the installer: `npx tauri build --no-bundle`
  (`src-tauri\target\release\tableboat.exe`).

Uninstalling (Settings > Apps, or `%LOCALAPPDATA%\Tableboat\uninstall.exe`) keeps the user data:
connections, settings and tab state in `%APPDATA%\Tableboat`, passwords in Windows Credential Manager.

### If packaging fails with "Access is denied (os error 5)"

The build downloads NSIS, unpacks it to `%LOCALAPPDATA%\tauri\nsis-3.13` and renames that to `NSIS`;
the antivirus can block the rename while it scans the new files. Finish the step by hand, then build again:

```powershell
$d = "$env:LOCALAPPDATA\tauri"
Rename-Item "$d\nsis-3.13" NSIS
$dir = "$d\NSIS\Plugins\x86-unicode\additional"; New-Item -ItemType Directory -Force $dir | Out-Null
Invoke-WebRequest -UseBasicParsing "https://github.com/tauri-apps/nsis-tauri-utils/releases/download/nsis_tauri_utils-v0.5.3/nsis_tauri_utils.dll" -OutFile "$dir\nsis_tauri_utils.dll"
(Get-FileHash "$dir\nsis_tauri_utils.dll" -Algorithm SHA1).Hash   # must be 75197FEE3C6A814FE035788D1C34EAD39349B860
```

(The URL and hash are the ones built into Tauri CLI 2.x at the time of writing; a newer CLI may want a newer plugin.)

## Release a new version

Releases are built by GitHub Actions (`.github/workflows/release.yml`), not on this PC, so every download is
built from the public source.

1. Pick the version ([Semantic Versioning](https://semver.org): `0.1.1` for fixes, `0.2.0` for new features).
2. Set it in all three files: `"version"` in `package.json` and `src-tauri/tauri.conf.json`, and `version` in
   `src-tauri/Cargo.toml` (then run `cargo build` once so `Cargo.lock` follows).
3. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD`, add a fresh empty
   `## [Unreleased]` above it, and update the links at the bottom. Write the entries for users: what they can
   do now, under `### Added`, `### Changed`, `### Fixed`, `### Removed`.
4. Commit, then tag and push:

   ```powershell
   git commit -am "chore: release x.y.z"
   git tag -a vx.y.z -m "Tableboat x.y.z"
   git push origin master vx.y.z
   ```

5. The Release workflow (about 15 minutes; Actions tab on GitHub) checks the version matches the tag in all
   three files, builds the installer, and creates a **draft** release with the installer, `SHA256SUMS.txt`, and
   notes made from the changelog section plus `.github/release-footer.md` (download, install, checksum).
6. Read the draft under Releases, edit if needed, and press **Publish release**.

The CI workflow (`.github/workflows/ci.yml`) type-checks, tests and builds every push and pull request.

## Measure memory and CPU

```powershell
powershell -ExecutionPolicy Bypass -File scripts\measure.ps1 -Exe "$env:LOCALAPPDATA\Tableboat\tableboat.exe"
```

Last result (v0.1.0, installed, connected to a local MongoDB): 143 MB private memory across 7 processes,
0.10 % idle CPU.
