# Installation

Flect is distributed as a native executable with no runtime model credential requirement. API credentials are needed only when a user later selects API-backed verification.

## Bootstrap installers

Tagged releases publish inspectable `install.sh` and `install.ps1` bootstrap installers. They select the current machine's supported target, download the matching archive over HTTPS, verify the exact archive entry in `SHA256SUMS`, validate the archive layout, and install only the executable. They do not modify shell profiles, user PATH settings, or system directories.

Supported targets are Linux x86-64 and arm64, macOS Intel and Apple silicon, and Windows x86-64. Other operating systems and architectures should use a native archive or build from source.

On Linux or macOS, download the script, inspect it, then run it:

```console
curl --fail --location --proto '=https' --proto-redir '=https' \
  https://github.com/aakbarpour/flect/releases/latest/download/install.sh \
  --output install.sh
sed -n '1,260p' install.sh
sh install.sh
```

The Unix installer defaults to `$HOME/.local/bin`. Override the destination or pin a release with `--bin-dir` and `--version`:

```console
sh install.sh --version v0.1.0 --bin-dir "$HOME/.local/bin"
```

On Windows PowerShell:

```powershell
Invoke-WebRequest `
  -Uri https://github.com/aakbarpour/flect/releases/latest/download/install.ps1 `
  -OutFile .\install.ps1
Get-Content .\install.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

The Windows installer defaults to `%LOCALAPPDATA%\Flect\bin`. Pin a release or select another user-local directory with `-Version` and `-BinDir`:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 -Version v0.1.0 -BinDir "$env:LOCALAPPDATA\Flect\bin"
```

Both installers support offline, already-downloaded release artifacts. Supply both archive and checksum paths; supplying only one is rejected:

```console
sh install.sh --version latest --archive ./flect-v0.1.0-x86_64-unknown-linux-gnu.tar.gz \
  --checksum-file ./SHA256SUMS --bin-dir "$HOME/.local/bin"
```

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1 `
  -Archive .\flect-v0.1.0-x86_64-pc-windows-msvc.zip `
  -ChecksumFile .\SHA256SUMS
```

If the destination is not already on PATH, the installer prints the exact shell or PowerShell assignment to use. Add it deliberately to your profile if you want it to persist. For environments that prohibit scripts, use the archive method below or install from source.

## Install the Codex repository plugin

After the executable is installed, `flect --version` succeeds, and the executable is on `PATH`, add the Flect repository marketplace and install its local plugin:

```console
codex plugin marketplace add aakbarpour/flect
codex plugin add flect@flect
```

The plugin bundles the Flect Skill and a stdio MCP configuration. The MCP configuration invokes `flect mcp` from `PATH`, so plugin installation does not download or replace the executable. Start a new Codex task after installation. To refresh a changed marketplace source, run `codex plugin marketplace upgrade flect`, reinstall with `codex plugin add flect@flect`, and start a new task.

If plugin marketplace support is unavailable, use the manual paths documented in [getting started](getting-started.md) and [the MCP guide](mcp.md).

## GitHub release archives

Each tagged release publishes five native archives, the two bootstrap installers, and `SHA256SUMS`:

| Platform | Target | Archive |
| --- | --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux arm64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS Apple silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows x86-64 | `x86_64-pc-windows-msvc` | `.zip` |

Archive names are `flect-vVERSION-TARGET.EXT`. Every archive contains the `flect` executable (`flect.exe` on Windows), `LICENSE`, and `README.md` inside a same-named directory.

The release workflow rejects a version tag that does not match the `flect-cli` Cargo package version. Manual workflow runs use `dev-RUN_NUMBER` artifact names and never publish a GitHub release.

Download the archive and `SHA256SUMS` from the release page. Verify before extracting:

```console
grep 'flect-v0.1.0-x86_64-unknown-linux-gnu.tar.gz$' SHA256SUMS | sha256sum --check
tar -xzf flect-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
install flect-v0.1.0-x86_64-unknown-linux-gnu/flect ~/.local/bin/flect
flect --version
```

On macOS, use the matching Darwin archive and pipe only its manifest line to `shasum -a 256 --check` when `sha256sum` is unavailable. The manifest also contains the installer hashes, so checking only the file you downloaded avoids false missing-file failures. On Windows PowerShell:

```powershell
Get-FileHash .\flect-v0.1.0-x86_64-pc-windows-msvc.zip -Algorithm SHA256
Expand-Archive .\flect-v0.1.0-x86_64-pc-windows-msvc.zip
.\flect-v0.1.0-x86_64-pc-windows-msvc\flect.exe --version
```

Compare the printed hash with the matching line in `SHA256SUMS`, then move the executable directory onto `PATH`.

## Build from source

Install stable Rust 1.85 or newer, then:

```console
git clone https://github.com/aakbarpour/flect.git
cd flect
cargo install --locked --path crates/flect-cli
flect --version
```

Flect does not yet publish Homebrew, Scoop, or other package-manager feeds. Those integrations intentionally follow validation of the native release artifacts and bootstrap installers.
