# Evaluation

Evaluation is a product requirement, not a demonstration exercise. Flect will measure correct-patch acceptance, bad-patch detection, false positives, per-class precision/recall, uncertainty, latency, token use, and estimated provider cost.

Milestone 1 contains deterministic reconciliation fixtures that exercise `SAME`, `PARTIAL`, `DIFFERENT`, and `UNCERTAIN`. These validate pipeline behavior; they are not evidence of model quality and no Flect effectiveness numbers are claimed yet.

Milestone 5 will add repository fixtures for correct changes, partial work, scope creep, constraint violations, semantic workarounds, broadened behavior, unrelated removals, missing edge cases, unnecessary refactors, and wrong-component fixes. Each fixture will retain the original task, base code, candidate patch, expected verdict, and important expected findings.

Provider comparisons will include a cheap verifier, cheap verification with escalation, and a stronger verifier. A no-Flect baseline must be defined precisely before comparison. Results will identify dataset construction, sample size, model versions, configuration, confidence intervals or uncertainty, failures, and estimated costs. RETRACE results will not be presented as Flect results.

