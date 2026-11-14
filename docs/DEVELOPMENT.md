# Developing Tableboat

All commands run from the project folder in a normal terminal (Rust's `cargo` must be on the PATH;
if a build says `cargo ... program not found`, open a new terminal). The same commands work on
Windows (PowerShell or Git Bash) and macOS.

## Run while developing

```sh
npx tauri dev
```

- The first start compiles the Rust side (a few minutes); later starts are much quicker.
- Changes under `src/` (React, CSS) show in the open window on save.
- Changes under `src-tauri/src/` recompile and restart the app.
- Dev mode uses the same connections, settings and saved passwords as the installed app.
- Developer tools: on Windows, run with WebView2's remote debugging and open
  `http://127.0.0.1:9222` in Edge or Chrome:

  ```powershell
  $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS="--remote-debugging-port=9222"; npx tauri dev
  ```

  On macOS, right-click anywhere in a development build and choose **Inspect Element**, or press
  ⌥⌘I.

## Code layout (Rust, `src-tauri/src/`)

- `lib.rs`: the window-free core `tableboat_core`, shared by both programs. It holds:
  - `conn`: saved connections, URLs, passwords;
  - `db`: engines, queries, results, the statement splitter, exports and dumps (`db/export.rs`);
  - `prefs`: settings;
  - `claude`: the Claude Code program and per-connection projects;
  - `mcp`: the tool server Claude uses, behind a `Host` trait: database tools in `mcp.rs`, app
    tools (settings, connections, exports) in `mcp_app.rs`; tools that change something ask the
    user through `Host::ask` themselves (`claude::ASKS_ITSELF`);
  - `paths`.
- `main.rs` with `ai`, `commands`, `settings`, `state`: the desktop app (Tauri). `ai/host.rs` plugs
  each window into the tool server.
- `bin/tb/`: the `tb` command line (see [CLI.md](CLI.md)).

```sh
cd src-tauri
cargo build --bin tb                 # just the command line (fast)
./target/debug/tb connections        # on Windows: .\target\debug\tb.exe connections
TABLEBOAT_DATA_DIR=/tmp/tb-test ./target/debug/tb connections   # without touching your real data
```

## Checks

```sh
npm run typecheck                    # TypeScript
(cd src-tauri && cargo test)         # Rust (live DB tests run only when their URLs are set, see src-tauri/src/db/mod.rs)
node scripts/test-splitter.ts        # SQL statement splitter
node scripts/test-readonly.ts        # read-only query detection (refresh / re-run)
```

## Build the installer

```sh
npx tauri build
```

- About 6-7 minutes on Windows; a little longer on an Apple Silicon Mac.
- Windows: `src-tauri\target\release\bundle\nsis\Tableboat_<version>_x64-setup.exe`. It installs for
  the current Windows user only (no admin prompt) into `%LOCALAPPDATA%\Tableboat`: the app and
  `tb.exe`, and adds that folder to the user PATH (`src-tauri\windows\hooks.nsh`; uninstalling
  takes it off again).
- macOS: `src-tauri/target/release/bundle/dmg/Tableboat_<version>_aarch64.dmg` and the app under
  `.../bundle/macos/Tableboat.app`. The app is ad-hoc signed (`"signingIdentity": "-"` in
  `src-tauri/tauri.conf.json`), which Apple Silicon requires; it isn't notarized yet.
- Running a newer setup upgrades in place (on macOS the new app replaces the old one). To change the
  version the OS shows, edit `"version"` in `src-tauri/tauri.conf.json` first.
- The app alone, without an installer: `npx tauri build --no-bundle`
  (`src-tauri\target\release\tableboat.exe`, or `src-tauri/target/release/tableboat` on macOS).

Uninstalling keeps the user data: connections, settings and tab state in `%APPDATA%\Tableboat`
(Windows) or `~/Library/Application Support/Tableboat` (macOS), passwords in Windows Credential
Manager or the macOS Keychain.

### macOS window chrome

`src-tauri/tauri.macos.conf.json` overrides the window for macOS: native decorations with
`titleBarStyle: "Overlay"`, so the system's traffic lights sit over the custom 40 px title bar
(`src/shell/TitleBar.tsx` adds left padding on macOS and hides the Windows-style caption buttons).
Tauri merges this file automatically when building on macOS.

### If Windows packaging fails with "Access is denied (os error 5)"

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

Releases are built by GitHub Actions (`.github/workflows/release.yml`), not on your own machine, so
every download is built from the public source. One tag builds both platforms.

1. Pick the version ([Semantic Versioning](https://semver.org): `0.1.1` for fixes, `0.2.0` for new features).
2. Set it in all three files: `"version"` in `package.json` and `src-tauri/tauri.conf.json`, and `version` in
   `src-tauri/Cargo.toml` (then run `cargo build` once so `Cargo.lock` follows).
3. In `CHANGELOG.md`, rename `## [Unreleased]` to `## [x.y.z] - YYYY-MM-DD`, add a fresh empty
   `## [Unreleased]` above it, and update the links at the bottom. Write the entries for users: what they can
   do now, under `### Added`, `### Changed`, `### Fixed`, `### Removed`.
4. Commit, then tag and push:

   ```sh
   git commit -am "chore: release x.y.z"
   git tag -a vx.y.z -m "Tableboat x.y.z"
   git push origin master vx.y.z
   ```

5. The Release workflow (about 20 minutes; Actions tab on GitHub) checks the version matches the tag
   in all three files, then builds on `windows-latest` and `macos-latest` (Apple Silicon), and creates a
   **draft** release with:
   - the Windows installer and its signature,
   - the macOS DMG and the updater's `.app.tar.gz` and its signature,
   - `SHA256SUMS.txt` for all of them,
   - notes made from the changelog section plus `.github/release-footer.md`,
   - `latest.json`, the update feed with both platforms.
6. Read the draft under Releases, edit if needed, and press **Publish release**.

Publishing is what makes installed copies offer the update: they read `latest.json` from the **latest
published** release (`releases/latest/download/latest.json`), never from a draft or pre-release.

The CI workflow (`.github/workflows/ci.yml`) type-checks, tests and builds every push and pull request,
on Windows and macOS.

### Signing

- **Updater:** every build is signed with the updater key (below); installed copies only accept an
  update whose signature matches the public key in `src-tauri/tauri.conf.json`
  (`plugins.updater.pubkey`).
- **Apple:** the macOS app is ad-hoc signed only. Without an Apple Developer ID and notarization,
  Gatekeeper asks the user to right-click → Open the first time (see the release notes). To sign
  properly later: set `bundle.macOS.signingIdentity` to the Developer ID, add
  `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
  `APPLE_PASSWORD` and `APPLE_TEAM_ID` as repository secrets, and Tauri notarizes during the build.

### The updater key

- Private key: `%USERPROFILE%\.tauri\tableboat-updater.key` on the maintainer's Windows PC (no
  password), and the GitHub repository secret `TAURI_SIGNING_PRIVATE_KEY` (its contents) for the
  Release workflow. On macOS the default location is `~/.tauri/tableboat-updater.key`.
- **Keep a backup of the key** (e.g. in a password manager). Without it, no new version can be signed for
  existing installs: users would have to download the next installer by hand once, after a new key and
  public key are set.
- Local builds don't need the key: `npx tauri build` makes the installer without updater files. To make a
  signed one locally (as the workflow does):

  ```sh
  export TAURI_SIGNING_PRIVATE_KEY="$(cat "$HOME/.tauri/tableboat-updater.key")"
  npx tauri build --config src-tauri/tauri.release.conf.json
  ```

- `TABLEBOAT_UPDATE_URL` points a running copy at another feed (for testing updates).

## Measure memory and CPU

On Windows:

```powershell
powershell -ExecutionPolicy Bypass -File scripts\measure.ps1 -Exe "$env:LOCALAPPDATA\Tableboat\tableboat.exe"
```

Last Windows result (v0.1.0, installed, connected to a local MongoDB): 143 MB private memory across 7
processes, 0.10 % idle CPU.
