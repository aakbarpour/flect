# Architecture

Flect's initial workspace contains three crates because each has a distinct dependency direction.

```text
flect-cli ─────> flect-core
    │                ▲
    └──> flect-runner┘ (shared serialized values only)
```

`flect-core` owns stable domain models, strict configuration, versioned run state, read-only Git capture, deterministic context selection, BlindGuard, and deterministic reconciliation. It does not depend on terminal rendering, a provider SDK, HTTP, Codex, or environment variables.

`flect-runner` owns the object-safe provider boundary. `AgentRunner` consumes a narrow `AgentRequest` and JSON Schema and produces JSON for typed deserialization by its caller. Keeping the trait non-generic permits future dynamic provider selection. Milestone 1 includes only `MockRunner`; async and HTTP are not introduced until a real provider requires them.

`flect-cli` owns argument parsing, filesystem-oriented command orchestration, terminal/JSON reports, and logging setup. It is the composition root, not a second domain layer.

## Verification flow

1. `start` discovers the worktree, resolves `HEAD`, captures the raw task, and persists a conservative `IntendedSpec`.
2. `verify` reloads the immutable run and captures the working tree relative to its base commit.
3. `ContextBuilder` removes sensitive/binary paths and deterministically selects focused context.
4. `BlindGuard` fails closed if task-bearing Git metadata is configured for inclusion and creates the only payload accepted by a verifier.
5. A runner returns `EchoedSpec`. In Milestone 1 this is a deterministic fixture or explicitly uncertain file-level mock.
6. Reconciliation compares the intended and echoed specifications, attaches evidence, and persists a `VerificationRecord`.

## State format

Project-local state is stored beneath `.flect/`:

```text
.flect/
├── latest
├── runs/<run-id>.json
└── results/<run-id>.json
```

Documents carry a `version` field. A run stores the repository root, immutable base revision, original task, forward spec, and creation timestamp. This sensitive task state remains local and is never copied into a blind bundle. Result files contain the sanitized bundle, echoed spec, verdict, and timestamp.

## Deliberate limits

Repository-copy context, network providers, model repair loops, cheap-model escalation, Skills, evaluation execution, MCP, CI mode, and release packaging are later milestones. No empty interfaces or placeholder crates exist for them.

