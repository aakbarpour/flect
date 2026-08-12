# Codex MCP integration

Flect exposes its application workflows as a local stdio Model Context Protocol server. Automated tools delegate to the existing machine-readable CLI path; agent handoff tools call `flect-app::AgentService` directly. Both converge on the same domain types, evidence policy, and versioned project-local `RunStore`.

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
- `flect_prepare_blind` creates sanitized read-only resources and a typed fresh-verifier job.
- `flect_submit_echo` validates and accepts one typed verifier response.
- `flect_prepare_reconciliation` creates the contract for a different fresh judge.
- `flect_submit_verdict` validates evidence, closes the judge job, and persists the result.
- `flect_get_result` retrieves that persisted result.

State-changing tools write only Flect's project-local `.flect/` state and prepared agent resources under the operating-system temporary directory. Flect's Git access remains read-only. The MCP process writes protocol messages to stdout and diagnostics to stderr.

## Protocol lifecycle and errors

Flect implements protocol version `2025-11-25`. A client must send `initialize`, receive the response, then send the `notifications/initialized` notification. `tools/list` and `tools/call` before that sequence return JSON-RPC error `-32002`; repeated `initialize` requests return `-32600`. Notifications never receive responses.

Malformed JSON, invalid requests, unknown methods, and invalid parameter shapes use JSON-RPC errors. Once a tool call has structurally valid arguments, repository, job-lifecycle, submission, and evidence failures are returned as normal MCP tool results with `isError: true` and structured error content. This distinction lets clients repair a tool workflow without treating it as a broken protocol session.

## Isolation and model reporting

API mode is strict with respect to the parent conversation because the configured remote backward request contains only the sanitized `BlindBundle`. Codex-native mode prepares resources outside the repository and removes the task, forward spec, parent conversation, Git metadata, `.git`, and `.flect`, but it reports `structural` isolation: a shared runtime filesystem may still permit a child to inspect other paths.

The orchestrator must create the verifier with a real no-parent-context spawn option and use a different fresh child for reconciliation. Record the actual model and whether selection was explicit, inherited, or unknown. Flect does not infer model entitlement or claim a particular child model from MCP configuration alone.
