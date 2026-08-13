# Codex-native agent mode

## Verifier spawn

Use the runtime's real collaboration tool. Prefer a child with no inherited turns or parent conversation. In runtimes exposing `spawn_agent(fork_turns=...)`, use `fork_turns="none"`.

Construct the child message only from `BlindAgentJob.instructions` and the paths in `allowed_resources`. The child may read only the prepared blind workspace. Do not mention the original task, IntendedSpec, issue, acceptance criteria, parent plan, branch, commits, or why a particular implementation was chosen.

The verifier invokes only Flect's repository-independent typed verifier lifecycle on its blind job: begin, set objective, add zero or more before/after/scope/side-effect/assumption/uncertainty values, set confidence, and submit. It writes no JSON and never needs repository access. Each `affected_scope` item is `{ "file": "exact/visible/path", "symbol": "optional detail or null" }`; only `file` is a filesystem reference and Flect validates it at the typed boundary. The parent passes only the completed job ID to `flect agent verifier-commit`; Flect constructs EchoedSpec, binds the job, validates its lifecycle, and persists acceptance. Record the actual model and whether it was explicitly selected, inherited, or unknown. Treat the child as untrusted until Flect accepts the submission.

## Judge spawn

After Flect accepts the echo, prepare a reconciliation job and spawn a different fresh child. The judge may receive the job's IntendedSpec, EchoedSpec, available evidence, `evidence_ref_contract`, and instructions. It must not receive the parent conversation or implementation reasoning. It invokes `judge-begin`, `judge-set-alignment`, repeated `judge-add-finding --text-file`, `judge-set-confidence`, and `judge-submit` itself. It provides only alignment, confidence, and findings (`kind`, text, optional `evidence_ref`). The parent only observes completion and never translates semantic output. Flect derives the submission envelope, persisted finding IDs, files, exact hunks, line ranges, and recommended action; `SAME` has no findings.

No judge-authored JSON or chat is parsed. Unknown categories are rejected by the typed command surface; fabricated evidence, invalid lifecycle use, and semantic invariant failures are rejected by Flect.

## Repair

Any patch edit invalidates the practical usefulness of prior reasoning. Prepare a new blind job and spawn new verifier and judge children after rerunning repository checks.
