# Flect

**Hide the prompt. Read the patch.**

Coding agents usually verify implementations while knowing what they were supposed to implement. Flect removes that anchor.

It gives an independent verifier the patch without the original task, reconstructs what the implementation actually appears to do, then compares that reconstructed intent with what you requested.

Tests ask whether the code works. Flect asks whether you built the right thing.

> **Project status:** Milestones 0, 1, and the core Milestone 2 semantic pipeline are implemented. Flect can run deterministic offline verification or use an OpenAI-compatible Responses endpoint for forward analysis, structurally blind reconstruction, semantic reconciliation, and bounded model escalation. Integrations, evaluation, and release packaging remain in progress.

## How it works

```text
Original task ──> IntendedSpec ───────────────┐
                                              ├──> reconciliation ──> verdict
Patch ──> BlindGuard ──> blind reconstruction┘
          (no original task)
```

BlindGuard makes task separation structural: the bundle type contains patch evidence, selected context, a manifest, and a blindness report—no task, conversation, forward spec, branch name, or commit message. It cannot guarantee that source code or comments do not themselves reveal task semantics; Flect reports that limitation rather than implying a cryptographic guarantee.

## Build

Flect uses stable Rust and the 2024 edition.

```console
cargo build --release
cargo test --workspace
```

The binary is `target/release/flect` (`flect.exe` on Windows).

## Quick start

Initialize configuration explicitly. This is the only command that adds `.flect/` to `.gitignore`.

```console
$ flect init
Flect initialized

Repository  /work/example
Config      /work/example/flect.toml (created)
State       .flect/ (added to .gitignore)
```

Capture the task before implementation changes can bias forward analysis:

```console
$ flect start --task "Reject expired refresh tokens without removing the legacy fallback"
Flect run created

Run      fl_8ea9c89f44170a36
Base     a81cc2d1
Task     captured
Spec     captured deterministically

Ready for implementation.
```

After making the change and running the repository's normal tests:

```console
$ flect inspect
Verifier bundle

Context      focused
Payload      18422 bytes
Patch files  2
  src/auth.rs
  tests/auth.rs

$ flect verify --echoed-spec fixtures/my-echoed-spec.json
Flect

Patch
  2 files
  +41 / -9

Alignment

  PARTIAL

Recommended action

  REVISE PATCH
```

`--echoed-spec` is an offline Milestone 1 seam, primarily for deterministic tests and evaluation fixtures. Omitting it uses the bundled mock runner and produces `UNCERTAIN`. Real provider configuration is not silently simulated.

## Responses API transport

Flect includes a reusable OpenAI-compatible Responses API runner with strict JSON Schema output, credential redaction, timeouts, and typed provider errors. Configure it in `flect.toml`:

```toml
[runner]
kind = "api"
protocol = "responses"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-5.6-luna"
fallback_model = "gpt-5.6-terra"
reasoning_effort = "medium"
timeout_seconds = 120
escalate_on_uncertain = true
confidence_threshold = 0.65
complexity_file_threshold = 12
complexity_byte_threshold = 200000
```

Set the named environment variable outside the configuration file. `flect doctor` reports whether it is present without printing its value. With `kind = "api"`, `flect start` generates the forward specification, `flect echo` performs blind reconstruction, and `flect verify` performs blind reconstruction followed by semantic reconciliation. Provider, model, latency, and reported token usage are persisted with the relevant run or verification result; credential values are not.

Before making a paid request, inspect the exact privacy boundary and runner selection:

```console
flect verify --dry-run
flect --json verify --dry-run
```

Dry-run output includes the provider, model, context policy, included patch/context files, and excluded paths. It never initializes the API runner or reads the credential value. See [model routing and cost estimates](docs/model-routing.md) for fallback signals, request limits, and the versioned pricing table.

## Commands

- `flect init` — install strict defaults and ignore local state.
- `flect start` — record the original task, base commit, and conservative `IntendedSpec`.
- `flect inspect` — print the exact sanitized verifier bundle without invoking a runner.
- `flect verify` — reconstruct intent and persist a structured verdict.
- `flect echo [REVISION]` — describe a patch without needing an original task.
- `flect doctor` — check Git, repository discovery, configuration, and runner readiness.

Every command supports `--json`. Use `--verbose` or `--verbose --verbose` for internal diagnostics; complete model payloads are not logged.

## Design and safety

- Git access is read-only. Flect never commits, stages, resets, checks out, changes branches, or edits Git configuration.
- `.env`, private keys, credential/secret paths, binaries, build output, vendored code, and dependency directories are excluded before bundle construction.
- Focused context contains changed file contents and a small deterministic set of root manifests, subject to byte limits.
- State is versioned JSON under the project-local `.flect/` directory. Credentials are never stored there.
- Normal tests use `MockRunner` and require no API key.

Read [the architecture](docs/architecture.md), [blind verification boundary](docs/blind-verification.md), and [privacy model](docs/privacy.md) before extending provider behavior.

## Research attribution

Flect is inspired by the reconstruct-and-verify method described in **Independent Patch Verification for Coding Agents with a Bidirectional Reconstruct-and-Verify Framework** (RETRACE), arXiv:2608.08950. Flect does not claim to have invented that academic method. See [Research and attribution](docs/research.md) for full authorship and a separation of research concepts from Flect-specific engineering.

## License

Flect is available under the [MIT License](LICENSE).
