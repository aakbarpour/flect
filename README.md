# Flect

**Hide the prompt. Read the patch.**

Coding agents usually verify implementations while knowing what they were supposed to implement. Flect removes that anchor.

It gives an independent verifier the patch without the original task, reconstructs what the implementation actually appears to do, then compares that reconstructed intent with what you requested.

Tests ask whether the code works. Flect asks whether you built the right thing.

> **Project status:** Milestones 0 and 1 are implemented. The CLI, Git capture, privacy boundary, structured domain, offline fixtures, and deterministic reconciliation are usable. A real model-backed verifier is deliberately deferred to Milestone 2. Without an explicit offline `EchoedSpec`, Flect returns an uncertain file-level reconstruction and never claims verification.

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
