# Architecture

Flect's initial workspace contains three crates because each has a distinct dependency direction.

```text
flect-cli ─────> flect-core
    │                ▲
    └──> flect-runner┘ (shared serialized values only)
```

`flect-core` owns stable domain models, strict configuration, versioned run state, read-only Git capture, deterministic context selection, BlindGuard, and deterministic reconciliation. It does not depend on terminal rendering, a provider SDK, HTTP, Codex, or environment variables.

`flect-runner` owns the object-safe provider boundary. `AgentRunner` consumes a narrow `AgentRequest` and JSON Schema and produces schema-validated JSON plus safe usage metadata. `MockRunner` is deterministic; `OpenAiResponsesRunner` implements async Responses-compatible HTTP without leaking provider dependencies into the core domain.

`flect-cli` owns argument parsing, filesystem-oriented command orchestration, terminal/JSON reports, and logging setup. It is the composition root, not a second domain layer.

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

Repository-copy context, model repair loops, cheap-model escalation, Skills, evaluation execution, MCP, CI mode, and release packaging are later milestones. No empty interfaces or placeholder crates exist for them.
