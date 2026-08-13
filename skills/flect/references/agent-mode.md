# Codex-native agent mode

## Verifier spawn

Use the runtime's real collaboration tool. Prefer a child with no inherited turns or parent conversation. In runtimes exposing `spawn_agent(fork_turns=...)`, use `fork_turns="none"`.

Construct the child message only from `BlindAgentJob.instructions` and the paths in `allowed_resources`. The child may read only the prepared blind workspace. Do not mention the original task, IntendedSpec, issue, acceptance criteria, parent plan, branch, commits, or why a particular implementation was chosen.

The verifier never executes Flect and never writes JSON. Flect creates an external draft root containing primitive files: `objective`, `confidence`, `submitted`, numbered text directories (`behavior_before`, `behavior_after`, `side_effects`, `assumptions`, `uncertainties`), and numbered `affected_scope/<index>/file` plus optional `symbol`. The child writes only these values and creates the empty `submitted` marker last. The parent passes only the completed job ID to the job-bound verifier commit; Flect validates the draft, constructs EchoedSpec, binds the job, and persists acceptance. Record the actual model and whether it was explicitly selected, inherited, or unknown. Treat the child as untrusted until Flect accepts the submission.

## Judge spawn

After Flect accepts the echo, prepare a reconciliation job and spawn a different fresh child. The judge writes only external draft primitives: one alignment marker directory, `confidence`, numbered finding directories containing one kind marker, `text`, and optional `evidence_ref`, plus a disposition for every `side_effect/<n>` candidate. A disposition is either `finding` (text and evidence_ref) or `not-distinct` (a non-empty reason). It creates `submitted` last. A `DIFFERENT` submission requires a `missing_requirement` or `unrequested_change` finding; violations and side effects alone do not establish an objective mismatch. The parent only observes completion and passes the job ID; Flect validates evidence, side-effect dispositions, lifecycle, and semantic invariants before constructing and persisting the verdict.

No judge-authored JSON or chat is parsed. Unknown categories are rejected by the typed command surface; fabricated evidence, invalid lifecycle use, and semantic invariant failures are rejected by Flect.

## Repair

Any patch edit invalidates the practical usefulness of prior reasoning. Prepare a new blind job and spawn new verifier and judge children after rerunning repository checks.
