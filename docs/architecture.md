# Architecture

Flect's workspace contains four crates because each has a distinct dependency direction.

```text
flect-cli ─────> flect-app ─────> flect-core
    │                               ▲
    └──────────> flect-runner ──────┘ (shared serialized values only)
```

`flect-core` owns stable domain models, strict configuration, versioned run state, read-only Git capture, deterministic context selection, BlindGuard, and deterministic reconciliation. It does not depend on terminal rendering, a provider SDK, HTTP, Codex, or environment variables.

`flect-runner` owns the object-safe provider boundary. `AgentRunner` consumes a narrow `AgentRequest` and JSON Schema and produces schema-validated JSON plus safe usage metadata. `MockRunner` is deterministic; `OpenAiResponsesRunner` implements async Responses-compatible HTTP without leaking provider dependencies into the core domain.

`flect-app` owns trusted adapter-neutral workflows. It prepares read-only structurally isolated agent resources, binds submissions to job/run lifecycle state, rejects fabricated scope and evidence, and converges Codex-native submissions on the same `EchoedSpec`, `Verdict`, and `VerificationRecord` artifacts as API mode.

`flect-cli` owns argument parsing, filesystem-oriented command orchestration, terminal/JSON reports, logging setup, and the thin stdio MCP adapter. MCP tool execution delegates to the existing machine-readable CLI commands so it cannot drift into a second verification implementation.

## Dual semantic execution

API mode asks configured Responses-compatible models to perform forward analysis, blind reconstruction, and reconciliation. Codex-native mode delegates only the two reasoning steps through typed jobs: a fresh blind verifier receives sanitized read-only resources, and a separate fresh judge receives the accepted echo plus intended specification. Flect remains authoritative for Git capture, filtering, schemas, job binding, evidence validation, persistence, and assurance metadata.

Prepared agent workspaces live under the operating-system temporary directory rather than the repository. They exclude `.git`, `.flect`, the task, forward specification, and task-bearing metadata. Files are marked read-only, but this is reported as `structural` isolation because a shared Codex runtime may still allow a child to inspect paths outside that workspace.

## Verification flow

1. `start` discovers the worktree, resolves `HEAD`, captures the raw task, and persists an `IntendedSpec` produced either by the deterministic baseline or the configured semantic runner.
2. `verify` reloads the immutable run and captures the working tree relative to its base commit.
3. `ContextBuilder` removes sensitive/binary paths and deterministically selects focused context.
4. `BlindGuard` fails closed if task-bearing Git metadata is configured for inclusion and creates the only payload accepted by a verifier.
5. The configured runner returns `EchoedSpec` from only the serialized blind bundle. Offline mode uses a deterministic fixture or explicitly uncertain file-level baseline.
6. API mode semantically reconciles intended and echoed specifications; offline mode uses the conservative deterministic reconciler.
7. Flect removes any model-produced evidence location that cannot be tied back to an actual changed file and verbatim patch hunk, then persists the `VerificationRecord` and safe model-call metadata.

## State format

Project-local state is stored beneath `.flect/`:

```text
.flect/
├── latest
├── runs/<run-id>.json
└── results/<run-id>.json
```

Documents carry a `version` field. A run stores the repository root, immutable base revision, original task, forward spec, safe model-call metadata, and creation timestamp. This sensitive task state remains local and is never copied into a blind bundle. Result files contain the sanitized bundle, echoed spec, verdict, safe model-call metadata, and timestamp.

## Deliberate limits

Repository-copy context, model repair loops beyond one bounded fallback, evaluation execution, CI mode, and release packaging are later milestones. No empty interfaces or placeholder crates exist for them.
