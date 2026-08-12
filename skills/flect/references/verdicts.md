# Verdict handling

## SAME

Report alignment, tests, uncertainties, exclusions, evidence limits, isolation level, and actual model-selection status. SAME is not proof of correctness.

## PARTIAL

Group missing requirements, constraints, unexpected scope, and side effects. Fix only after understanding the finding. Rerun checks and create fresh verifier and judge jobs.

## DIFFERENT

Stop release activity and revisit the implementation interpretation. Do not patch isolated symptoms without reconciling the intended and apparent objectives. Repeat with fresh jobs after changes.

## UNCERTAIN

Make no positive verification claim. Review exclusions and context limits. Deliberately choose more context, a supported stronger child, configured API verification, or genuinely missing user information.

Never describe confidence as a calibrated probability.
