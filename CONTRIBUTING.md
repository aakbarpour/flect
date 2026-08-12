# Contributing to Flect

Thank you for helping build Flect. The project favors explicit, small, typed, and testable code over speculative abstraction.

## Development workflow

All non-trivial changes follow this sequence:

```text
Issue → branch → commits → pull request → CI → review → squash merge → main
```

1. Search the issue tracker before starting work. Reuse an existing issue when it covers the change; otherwise open one with a clear problem, scope, non-goals, acceptance criteria, and validation requirements.
2. Branch from an up-to-date `main`. Do not commit substantive feature work directly to `main`.
3. Name the branch `<kind>/<issue>-<short-name>`, where `kind` is `feat`, `fix`, `docs`, `test`, `chore`, or `refactor`.
4. Keep commits cohesive and use concise Conventional Commit-style subjects, such as `feat(runner): add Responses API support`.
5. Open a pull request targeting `main`. Complete the repository template and use `Closes #123` when the pull request fully resolves its primary issue.
6. Resolve review findings and make all required checks pass before marking a draft ready.
7. Squash merge the pull request. GitHub closes the linked issue after the merge and deletes the feature branch automatically.

Small typo and comment fixes may not need a dedicated issue when their scope and risk are self-evident, but they still use a pull request. Do not create retrospective bookkeeping issues for work that is already complete.

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

## Pull request readiness

Before marking a pull request ready for review, run:

```console
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```

Also run feature-specific integration tests. Normal validation must not make paid model calls unless the test is explicitly opt-in and documented as such.

`main` is the canonical integration branch. Force pushes and branch deletion are prohibited. Repository administrators should require pull requests and the CI matrix before merge; see [the governance guide](docs/governance.md) for the recommended GitHub settings.
