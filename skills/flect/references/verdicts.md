# Verdict handling

## SAME

State that the reconstructed patch intent aligns with the captured specification at the reported confidence. Also report relevant test status, uncertainties, excluded context, and any evidence limitations. `SAME` is not proof of correctness or completeness.

## PARTIAL

Group findings into missing requirements, violated constraints, unrequested scope, and possible side effects. Check each location-backed claim against the cited patch. Revise only after understanding whether the patch or captured specification is wrong, then rerun repository checks and `flect verify`.

## DIFFERENT

Treat the patch as implementing a materially different objective. Stop release or merge activity. Compare the original task, `IntendedSpec`, reconstructed objective, and evidence. Revisit the implementation approach rather than patching isolated symptoms.

## UNCERTAIN

Make no positive verification claim. Inspect the BlindGuard report, excluded paths, context limits, provider errors, and model-routing attempts. Use `flect verify --dry-run` before broadening disclosure. Ask the user before materially increasing context or making a paid request that was not already authorized.

## Assurance language

- Say “structurally blind” only for Flect's API-backed backward request.
- Say “offline baseline” for mock output.
- Never describe confidence as calibrated probability.
- Never imply that the active Codex conversation is hidden from itself.
