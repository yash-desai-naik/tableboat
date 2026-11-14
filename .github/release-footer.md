---

## Download

| File | For |
|---|---|
| [**{file}**](https://github.com/{repo}/releases/download/v{version}/{file}) | Windows 10 and 11, 64-bit |

**Install:** run the installer. It installs for your Windows user only (no administrator rights) into
`%LOCALAPPDATA%\Tableboat`, and adds the `tb` command to your PATH (open a new terminal to use it).
A newer installer upgrades in place; your connections, settings and saved passwords are kept.

> [!NOTE]
> The installer is not code-signed yet, so Windows SmartScreen may show "Windows protected your PC".
> Click **More info**, then **Run anyway**. You can check that the file is the one built here with its
> SHA-256 checksum below.

**SHA-256** (also in `SHA256SUMS.txt`):

```
{sha256}  {file}
```

Check it in PowerShell: `(Get-FileHash .\{file}).Hash`

**Requirements:** Windows 10 or 11, 64-bit, with Microsoft Edge WebView2 (built into Windows 11; on
Windows 10 the installer downloads it if it's missing). Optional:
[Claude Code](https://claude.com/claude-code), signed in, for the Claude pane and `tb ask`.

**Uninstall:** Settings > Apps > Tableboat. Your connections and settings stay in `%APPDATA%\Tableboat`
and your passwords in Windows Credential Manager, in case you reinstall.

This build was made by GitHub Actions from the source code at tag `v{version}`.
Full history: [CHANGELOG.md](https://github.com/{repo}/blob/v{version}/CHANGELOG.md).
