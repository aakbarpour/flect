---
name: flect
description: Orchestrate Flect's independent intent-verification lifecycle in a Git repository. Use when Codex should capture an implementation task before coding, inspect Flect's privacy boundary, run blind patch verification, or respond to SAME, PARTIAL, DIFFERENT, or UNCERTAIN verdicts. Do not use as a substitute for tests, code review, or Flect's own verifier.
---

# Flect

Use the `flect` CLI as the source of verification behavior. Do not reproduce its prompts, reconciliation, or evidence logic in this skill.

## Workflow

1. Run `flect doctor`. Resolve configuration, Git, or credential failures before continuing.
2. Before implementation changes, capture the user's task faithfully:

   ```console
   flect start --task "<original task>"
   ```

   Use `--task-file` for long tasks. Do not paraphrase away constraints. Use `--spec-file` only when the user supplied or approved that structured specification.
3. Implement the task and run the repository's normal checks. Flect supplements those checks; it does not replace them.
4. Before a paid or external request, run `flect verify --dry-run`. Review the provider, model, context policy, included files, excluded files, and BlindGuard report. Stop if the disclosure is broader than the user expects.
5. Run `flect verify` from the same worktree. Use `--echoed-spec` only for explicit offline fixtures or tests.
6. Handle the verdict:
   - `SAME`: report alignment and remaining test/review caveats. Do not call it proof of correctness.
   - `PARTIAL`: inspect missing requirements, constraints, scope changes, and evidence; revise the patch and verify again.
   - `DIFFERENT`: stop shipping work, compare the implementation with the captured task, and revisit the approach.
   - `UNCERTAIN`: do not claim verification. Review excluded context and runner readiness, then request more context or an explicit API-backed run.

Read [references/verdicts.md](references/verdicts.md) when handling a non-`SAME` result or explaining assurance boundaries.

## Isolation rules

- Treat `flect inspect` and `flect verify --dry-run` as disclosure inspection, not verification.
- Never add the original task, forward specification, conversation, branch name, issue text, or commit messages to a backward-verifier payload.
- Distinguish orchestration from verification. The active Codex agent knows the task and is not a blind verifier. Strict verification occurs only when Flect's configured API runner sends its structural `BlindBundle` to the provider.
- Do not claim that Codex selected a separate model, session, or hidden context unless a documented product capability proves it. Mock mode is an offline baseline and normally returns `UNCERTAIN`.
- Preserve Flect's evidence caveats and model-routing labels. Confidence is advisory, not a calibrated probability.
