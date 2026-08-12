# Codex MCP integration

Flect exposes its existing application pipeline as a local stdio Model Context Protocol server. The adapter does not reimplement verification policy: tool calls delegate to the same structured CLI commands, and stored results use the same versioned project-local `RunStore`.

## Configure Codex

Build Flect, then register the absolute binary path with the current Codex CLI:

```console
cargo build --release
codex mcp add flect -- C:\path\to\flect\target\release\flect.exe mcp
codex mcp list
```

On macOS or Linux, use the corresponding `target/release/flect` path. Codex stores MCP configuration in `~/.codex/config.toml`. A trusted repository may instead carry project-scoped configuration in `.codex/config.toml`:

```toml
[mcp_servers.flect]
command = "C:\\path\\to\\flect\\target\\release\\flect.exe"
args = ["mcp"]
cwd = "C:\\path\\to\\your\\repository"
```

The `cwd` must be the Git worktree Flect should inspect. Environment variables required by an API runner can be passed with Codex MCP configuration's `env` or `env_vars` settings; keep credential values outside committed files. Use `/mcp` in Codex to confirm the server is active.

## Tools and trust boundary

- `flect_start` captures the original task and immutable base revision.
- `flect_inspect` returns the exact strict blind verifier bundle.
- `flect_echo` reconstructs apparent patch intent without the original task.
- `flect_verify` reconstructs, reconciles, and persists a structured verdict.
- `flect_get_result` retrieves that persisted result.

`flect_start` and `flect_verify` write only Flect's project-local `.flect/` state. Flect's Git access remains read-only. The MCP process writes protocol messages to stdout and diagnostics to stderr; operational failures are returned as MCP tool errors, while malformed JSON-RPC uses standard protocol errors.

The strict blindness claim applies to the serialized backward-verifier bundle, not to the Codex agent invoking the tool. The MCP adapter never adds the original task to `inspect`, `echo`, `verify`, or stored-result responses.
