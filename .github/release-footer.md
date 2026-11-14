---

## Download

| File | For |
|---|---|
| **Tableboat_{version}_x64-setup.exe** | Windows 10 and 11, 64-bit |
| **Tableboat_{version}_aarch64.dmg** | macOS 11 or later, Apple Silicon |

**Windows:** run the installer. It installs for your Windows user only (no administrator rights) into
`%LOCALAPPDATA%\Tableboat`, and adds the `tb` command to your PATH (open a new terminal to use it).
A newer installer upgrades in place; your connections, settings and saved passwords are kept.

**macOS:** open the DMG and drag Tableboat to Applications. The `tb` command lives inside the app
bundle; see the README to put it on your PATH. Updates install in place and keep your connections,
settings and saved passwords.

> [!NOTE]
> Neither build is code-signed yet. On Windows, SmartScreen may show "Windows protected your PC":
> click **More info**, then **Run anyway**. On macOS, the first time you open Tableboat, right-click
> it in Applications and choose **Open** (or run
> `xattr -dr com.apple.quarantine /Applications/Tableboat.app`), then confirm. You can check that a
> file is the one built here with its SHA-256 checksum below.

**SHA-256** (also in `SHA256SUMS.txt`):

```
{sha256}  Tableboat_{version}_x64-setup.exe
{sha256-mac}  Tableboat_{version}_aarch64.dmg
```

Check them in PowerShell: `(Get-FileHash .\Tableboat_{version}_x64-setup.exe).Hash`, or on macOS:
`shasum -a 256 Tableboat_{version}_aarch64.dmg`.

**Requirements:** Windows 10 or 11, 64-bit, with Microsoft Edge WebView2 (built into Windows 11; on
Windows 10 the installer downloads it if it's missing); or macOS 11 or later on Apple Silicon.
Optional: [Claude Code](https://claude.com/claude-code), signed in, for the Claude pane and `tb ask`.

**Uninstall:** on Windows, Settings > Apps > Tableboat; on macOS, move Tableboat to the Bin. Your
connections and settings stay in the app's data folder, and your passwords in the system keychain,
in case you reinstall.

This build was made by GitHub Actions from the source code at tag `v{version}`.
Full history: [CHANGELOG.md](https://github.com/{repo}/blob/v{version}/CHANGELOG.md).
