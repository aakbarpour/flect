# Installation

Flect is distributed as a native executable with no runtime model credential requirement. API credentials are needed only when a user later selects API-backed verification.

## GitHub release archives

Each tagged release publishes five archives:

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
sha256sum --check SHA256SUMS
tar -xzf flect-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
install flect-v0.1.0-x86_64-unknown-linux-gnu/flect ~/.local/bin/flect
flect --version
```

On macOS, use the matching Darwin archive and the same `shasum -a 256` digest algorithm when `sha256sum` is unavailable. On Windows PowerShell:

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

Flect does not yet publish Homebrew, Scoop, or other package-manager feeds. Those integrations intentionally follow validation of the native release artifacts.
