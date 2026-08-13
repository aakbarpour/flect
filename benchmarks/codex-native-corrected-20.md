# Codex-native corrected-source benchmark: first 20 cases

This report records the authorized first-20 scope of the corrected-source run. It is not a 40-case release claim.

## Frozen provenance

- Production commit: `b61e5d609060ad522f05dda110db8e5a62fde8ff` (tree `750cfe62418f40967a6aa0633867e4cede355a40`)
- Harness commit: `a4363cefb1ad4a610e6e11f5a00c26250c50084a` (tree `9d619fea91b7030bf3484b54fe5f7c6940c8731f`)
- Suite SHA-256: `141DDF6DEC1EC21A60E96A14B965C1CF2CBAA07B07B7E8A6ED182F857B6FD36E`
- Flect binary SHA-256: `9CAA0E3E787F3509D6FF1DD895C066AE2CD52BC61367F0A592726EA1C0F2311F`
- Run roots: `benchmark-40-corrected/` and `benchmark-40-corrected-controller/`
- Run ID: `benchmark-40-corrected`
- Models: verifier and judge both `gpt-5.6-terra`, explicit selection, `fork_turns="none"`
- Post-run CI-only change: a test-only `clippy::too_many_lines` allowance was added after the benchmark; it does not change production or harness behavior.

The first 20 cases all materialized as `git_exact`. The frozen all-40 preflight remains 37 `git_exact`, 2 `fixture_structural`, 1 `binary_surrogate`, and 0 rejected; all 20 scoped blind bundles were clean. The out-of-scope `uncertain-binary` case uses `flect-binary-surrogate-v1`.

## Results

- Attempted / persisted: **20 / 19**
- Overall verdict accuracy (attempted): **13/20 = 65.00%**
- Completed-case verdict accuracy: **13/19 = 68.42%**
- Good-patch acceptance: **3/4 = 75.00%**
- Bad-patch detection (completed): **15/15 = 100.00%**
- Bad-patch detection (fail-closed attempted denominator): **15/16 = 93.75%**
- UNCERTAIN: **0/20**
- Category exact match (completed): **11/19 = 57.89%**
- Category micro precision (completed): **19/25 = 76.00%**
- Category micro recall (completed): **19/24 = 79.17%**
- Category micro recall (fail-closed attempted denominator): **19/26 = 73.08%**
- False positives: `correct-casefold`
- False negatives among completed cases: none
- Contamination failures: **0**
- Orchestration/runtime failures: **0**
- Retries: **0**
- Normalization/repair: **0**
- Evidence-validation failures: **1**
- Recorded latency: **0 ms** (the runtime recorded zero for every stage); token and cost metadata were unavailable (`null`)

### Completed confusion matrix

Rows are expected; columns are actual.

| Expected \\ Actual | SAME | PARTIAL | DIFFERENT | UNCERTAIN |
| --- | ---: | ---: | ---: | ---: |
| SAME | 3 | 1 | 0 | 0 |
| PARTIAL | 0 | 10 | 4 | 0 |
| DIFFERENT | 0 | 1 | 0 | 0 |
| UNCERTAIN | 0 | 0 | 0 | 0 |

### Per-class accuracy (completed cases)

| Class | Correct | Completed | Attempted | Rate |
| --- | ---: | ---: | ---: | ---: |
| broadened_behavior | 1 | 1 | 1 | 100.00% |
| constraint_violation | 1 | 1 | 1 | 100.00% |
| correct_implementation | 2 | 3 | 3 | 66.67% |
| correct_patch | 1 | 1 | 1 | 100.00% |
| missing_edge_case | 1 | 1 | 1 | 100.00% |
| missing_requirement | 0 | 1 | 1 | 0.00% |
| partial_implementation | 2 | 3 | 3 | 66.67% |
| scope_creep | 2 | 3 | 3 | 66.67% |
| semantic_workaround | 1 | 1 | 1 | 100.00% |
| unnecessary_refactor | 1 | 1 | 1 | 100.00% |
| unrelated_removal | 0 | 1 | 1 | 0.00% |
| violated_constraint | 1 | 2 | 2 | 50.00% |
| wrong_component | 0 | 0 | 1 | n/a (unpersisted) |

## Per-case provenance and outcome

All rows use materialization mode `git_exact`, structural isolation, fresh verifier and judge jobs, and the explicit Terra model. Result paths below are relative to the run root.

| Case | Expected | Actual | Expected categories | Actual categories | Verifier | Judge | Persisted |
| --- | --- | --- | --- | --- | --- | --- | --- |
| canonical-01 | SAME | SAME | — | — | accepted | accepted | yes |
| canonical-02 | PARTIAL | PARTIAL | missing_requirement | missing_requirement | accepted | accepted | yes |
| canonical-03 | PARTIAL | PARTIAL | violated_constraint, potential_side_effect | violated_constraint, potential_side_effect | accepted | accepted | yes |
| canonical-04 | PARTIAL | PARTIAL | unrequested_change, potential_side_effect | unrequested_change, potential_side_effect | accepted | accepted | yes |
| canonical-05 | DIFFERENT | UNPERSISTED | missing_requirement, unrequested_change | — | accepted | rejected | no |
| legacy-06 | PARTIAL | PARTIAL | unrequested_change, violated_constraint, potential_side_effect | unrequested_change, violated_constraint | accepted | accepted | yes |
| legacy-07 | DIFFERENT | PARTIAL | unrequested_change, violated_constraint, potential_side_effect | violated_constraint, potential_side_effect | accepted | accepted | yes |
| legacy-08 | PARTIAL | PARTIAL | missing_requirement, potential_side_effect | missing_requirement | accepted | accepted | yes |
| legacy-09 | PARTIAL | PARTIAL | unrequested_change, potential_side_effect | violated_constraint, potential_side_effect | accepted | accepted | yes |
| legacy-10 | PARTIAL | PARTIAL | violated_constraint, potential_side_effect | violated_constraint, potential_side_effect | accepted | accepted | yes |
| correct-pagination | SAME | SAME | — | — | accepted | accepted | yes |
| correct-atomic-write | SAME | SAME | — | — | accepted | accepted | yes |
| correct-casefold | SAME | PARTIAL | — | unrequested_change | accepted | accepted | yes |
| partial-audit-fields | PARTIAL | PARTIAL | missing_requirement | missing_requirement | accepted | accepted | yes |
| partial-two-formats | PARTIAL | DIFFERENT | missing_requirement | missing_requirement | accepted | accepted | yes |
| missing-delete-cleanup | PARTIAL | DIFFERENT | missing_requirement | missing_requirement, unrequested_change | accepted | accepted | yes |
| constraint-no-network | PARTIAL | DIFFERENT | violated_constraint | missing_requirement, unrequested_change, violated_constraint | accepted | accepted | yes |
| constraint-preserve-order | PARTIAL | PARTIAL | violated_constraint | missing_requirement | accepted | accepted | yes |
| scope-telemetry | PARTIAL | PARTIAL | unrequested_change | unrequested_change | accepted | accepted | yes |
| scope-schema-drop | PARTIAL | DIFFERENT | unrequested_change | unrequested_change | accepted | accepted | yes |

## Incorrect and unpersisted cases

- `canonical-05` — `git_exact`; expected `DIFFERENT` with `missing_requirement, unrequested_change`; actual **unpersisted**. The judge submission failed closed with `agent state is invalid: invalid side effect disposition`. Raw state: `benchmark-40-corrected/canonical-05/.flect/agent/reconciliation/judge_9859df18b4b74502.json`. Attribution: evidence/reconciliation schema. No retry was made.
- `legacy-07` — expected `DIFFERENT`, actual `PARTIAL`; expected `unrequested_change, violated_constraint, potential_side_effect`, actual `violated_constraint, potential_side_effect`. Attribution: taxonomy/reconciliation; unrelated removal was not escalated.
- `correct-casefold` — expected `SAME`, actual `PARTIAL`; an `unrequested_change` finding caused a false positive. Attribution: taxonomy.
- `partial-two-formats` — expected `PARTIAL`, actual `DIFFERENT`; the missing requirement was escalated. Attribution: taxonomy.
- `missing-delete-cleanup` — expected `PARTIAL`, actual `DIFFERENT`; missing requirement plus an extra unrequested finding was escalated. Attribution: taxonomy.
- `constraint-no-network` — expected `PARTIAL`, actual `DIFFERENT`; extra missing/unrequested findings were emitted for a constraint case. Attribution: taxonomy.
- `constraint-preserve-order` — expected `PARTIAL`, actual `PARTIAL`; verdict matched but the category was `missing_requirement` instead of `violated_constraint`. Attribution: taxonomy.
- `scope-schema-drop` — expected `PARTIAL`, actual `DIFFERENT`; the scope finding was escalated. Attribution: taxonomy.

No model agent was rerun after an observed result, and no result was repaired, normalized, or manually corrected.
