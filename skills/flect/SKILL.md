---
name: flect
description: Orchestrate Flect verification in a Git repository through API-backed reasoning or Codex-native fresh subagents. Use when Codex should capture a task before coding, prepare a structurally blind verifier job, spawn a separate reconciliation judge, run automated API verification, or respond to SAME, PARTIAL, DIFFERENT, or UNCERTAIN.
---

# Flect

Use Flect as the trusted state, sanitization, schema, evidence, and persistence layer. Never make the parent implementation agent act as the blind verifier.

## Start and implement

1. Run `flect doctor`.
2. Before edits, call `flect_start` through MCP or run `flect start --task "<exact task>"`.
3. Implement the task and run the repository's normal checks.
4. Choose agent mode by default when the runtime can spawn a fresh child without inherited conversation. Use API mode when explicitly selected, agent spawning is unavailable, or the user requests configured API isolation.

## Agent mode

1. Call `flect_prepare_blind` or `flect agent prepare-blind`.
2. Spawn a fresh verifier with the runtime's no-parent-context option. In the current collaboration runtime, use `spawn_agent` with `fork_turns="none"`. Give it only the returned instructions and allowed read-only resources. Do not add the task, issue, plan, tests derived from intent, branch, commits, or parent reasoning.
3. Request only `EchoedSpec`. Prefer the configured default primary (`gpt-5.6-luna`) when the runtime accepts an explicit override; otherwise inherit and report the runtime model. Never claim Luna was used unless the spawn API accepted Luna and the result records it.
4. Submit the structured response with `flect_submit_echo` or `flect agent submit-echo`.
5. Call `flect_prepare_reconciliation` or `flect agent prepare-reconciliation`.
6. Spawn a different fresh judge with no inherited conversation. Give it only the returned judge contract. The judge itself invokes Flect's typed lifecycle: begin, set alignment, add zero or more findings, set confidence, and submit.
7. The parent only observes completion. It must not translate semantic output into arguments, JSON, or persisted state.

Read [references/agent-mode.md](references/agent-mode.md) before spawning agents and [references/isolation.md](references/isolation.md) when reporting assurance.

## API mode

Run `flect verify --dry-run`, review disclosure, then run `flect verify`. Read [references/api-mode.md](references/api-mode.md) for selection and paid-request boundaries.

## Verdict loop

- `SAME`: report alignment plus normal test/review caveats.
- `PARTIAL`: fix the findings, rerun project checks, and create new verifier and judge jobs. Never reuse child contexts.
- `DIFFERENT`: reconsider task interpretation before more edits, then repeat with fresh jobs.
- `UNCERTAIN`: do not claim verification passed. Deliberately adjust context, use a supported stronger child, use explicitly configured API verification, or request missing information.

Respect the configured maximum iterations. Read [references/verdicts.md](references/verdicts.md) for detailed handling.
