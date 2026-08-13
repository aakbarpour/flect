# Codex-native agent mode

## Verifier spawn

Use the runtime's real collaboration tool. Prefer a child with no inherited turns or parent conversation. In runtimes exposing `spawn_agent(fork_turns=...)`, use `fork_turns="none"`.

Construct the child message only from `BlindAgentJob.instructions` and the paths in `allowed_resources`. The child may read only the prepared blind workspace. Do not mention the original task, IntendedSpec, issue, acceptance criteria, parent plan, branch, commits, or why a particular implementation was chosen.

The verifier returns one JSON object matching `echoed_spec_schema`. Each `affected_scope` item is `{ "file": "exact/visible/path", "symbol": "optional detail or null" }`; only `file` is a filesystem reference. Record the actual model and whether it was explicitly selected, inherited, or unknown. Treat the child as untrusted until Flect accepts the submission.

## Judge spawn

After Flect accepts the echo, prepare a reconciliation job and spawn a different fresh child. The judge may receive the job's IntendedSpec, EchoedSpec, available evidence, `evidence_contract`, instructions, judge schema, submission file, and submission schema. It must not receive the parent conversation or implementation reasoning. It writes exactly one `ReconciliationAgentSubmission` containing `alignment`, `findings[{kind,text,evidence_ref}]`, and `confidence` to Flect's generated `submission_file`, then stops. The trusted orchestrator later invokes `flect agent submit-verdict --submission-file <submission_file>` or `flect_submit_verdict` using only that opaque Flect-generated path. The orchestrator must never read or parse the file, nor copy or parse judge chat. Flect derives persisted finding IDs, files, exact hunks, line ranges, and the recommended action; `SAME` has no findings.

Flect strictly parses the direct submission. Markdown fences, prose, JSON wrappers, unknown keys, fabricated evidence, mismatched job IDs, and reused jobs are rejected. Do not persist or act on an unvalidated verdict.

## Repair

Any patch edit invalidates the practical usefulness of prior reasoning. Prepare a new blind job and spawn new verifier and judge children after rerunning repository checks.
