# Contributing to Flect

Thank you for helping build Flect. The project favors explicit, small, typed, and testable code over speculative abstraction.

## Development setup

Install stable Rust and Git, then run:

```console
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Normal tests must not require network access or API credentials. Use `MockRunner` for runner behavior. Git integration tests should create temporary repositories and must never change the contributor's repository state or global Git configuration.

## Changes

- Keep `flect-core` independent of CLI, HTTP, OpenAI, terminal rendering, Codex, and environment variables.
- Add dependencies only when the standard library or an existing dependency cannot express the requirement clearly.
- Preserve the BlindGuard type boundary. Restricted task-bearing metadata must not enter `BlindBundle`.
- Add behavior-focused tests for parsing, filtering, persistence, and user-visible failure modes.
- Update the relevant document when changing a security, privacy, or architectural invariant.

Pull requests should explain the user-visible problem, the chosen boundary, verification performed, and any known limitation. Do not include generated benchmarks, unmeasured performance claims, or placeholder architecture.

