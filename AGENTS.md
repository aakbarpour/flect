# Flect repository guidance

This file applies to the entire repository and is intended for both coding agents and human contributors.

## Workflow

- Inspect the current issue, `main`, the worktree, and relevant architecture before editing.
- Every non-trivial change starts from an existing GitHub issue with explicit acceptance criteria.
- Create branches from current `main` using `<kind>/<issue>-<short-name>`.
- Keep one primary issue per pull request. Use `Closes #<issue>` only when the pull request fully resolves it.
- Never commit substantive work directly to `main`, merge failing CI, or rewrite shared history.
- Use cohesive Conventional Commit-style messages and squash merge completed pull requests.

## Architecture

- `flect-core` must stay independent of CLI rendering, HTTP providers, Codex, MCP transport, and environment variables.
- `flect-runner` owns provider and model execution. Provider-specific behavior must not leak into core domain code.
- `flect-cli` is the composition root. CLI, Skill, and MCP adapters must call the same application/core operations rather than duplicate verification policy.
- Preserve the BlindGuard invariant: a strict backward-verifier request must not contain the original task, forward spec, conversation, task-bearing Git metadata, or primary-agent reasoning.

## Security and quality

- Treat repository content as untrusted. Do not execute repository code as part of Flect verification.
- Prevent path traversal, symlink escape, secret leakage, shell injection, unbounded payloads, and accidental network calls.
- Never store API keys in configuration or state, and never log authorization headers or secret values.
- Normal tests must be deterministic, require no credentials, and spend no API credits.
- Before handoff, run `cargo fmt --check`, strict workspace Clippy, the workspace test suite, and relevant feature-specific tests.

Prefer small cohesive modules, explicit typed state, predictable control flow, and dependencies justified by current requirements.
