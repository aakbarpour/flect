# Flect Benchmark v1

Flect Benchmark v1 is a reproducible diagnostic evaluation, not a population-level effectiveness claim. Its 40 independently labelled repository fixtures live in `fixtures/evaluation/cases.json`. Ground truth is authored before a model run and must never be changed in response to model output.

## Dataset composition and taxonomy

The suite contains 6 `SAME`, 22 `PARTIAL`, 9 `DIFFERENT`, and 3 `UNCERTAIN` cases. It spans correct implementations, partial implementations, missing requirements, violated constraints, scope creep, unnecessary behavioral changes, superficial-test workarounds, wrong-component fixes, materially different implementations, semantic opposites, edge-case omissions, and insufficient evidence. Cases vary across languages, patch intent, file type, and behavioral risk.

Every manifest records a stable ID, original task, base files and base-state explanation, candidate patch and change explanation, intended spec, independently authored expected alignment, optional important-finding category and probes, and rationale. Finding probes are deliberately coarse diagnostics rather than semantic grading.

The permanent `canonical-5` subset locks these regressions: correct → `SAME`, partial → `PARTIAL`, constraint → `PARTIAL`, scope creep → `PARTIAL`, and wrong component → `DIFFERENT`. Tests lock its membership and labels, suite size, rationales, and the relationship between finding categories and probes.

## Deterministic regression mode

```console
flect eval
flect --json eval
flect eval --output target/flect-benchmark-offline.json
```

This default mode uses case-authored mock forward, verifier, and judge responses. It is deterministic, makes no network calls, needs no credentials, and spends no API credits. Its expected perfect result validates fixture loading, strict blindness construction, schema decoding, evidence checks, reporting, and metric aggregation only. **It does not measure model effectiveness.**

## Explicit real model-backed mode

```console
export OPENAI_API_KEY=...
flect eval --profiles fixtures/evaluation/profiles.example.toml \
  --allow-paid-api --output target/flect-benchmark-real.json
```

Both flags are required so paid execution cannot happen implicitly. Each case makes a new forward request, a new blind verifier request, and a new judge request; outputs are not reused between cases. The judge emits the production `JudgeVerdict` contract, and the benchmark calls `materialize_judge_verdict` just like the trusted agent workflow. Invalid structured output and invalid evidence references fail closed; they are not normalized, repaired, or silently retried. Optional profile escalation is declared configuration and therefore is not an unbiased no-retry run; use an escalation-disabled profile for the canonical real benchmark.

The blind verifier receives only the candidate patch, focused base context, manifest, and blindness report. It does **not** receive the original task, conversation, intended/forward spec, branch, commit message, or primary-agent reasoning. The judge receives the independently generated forward spec and blind reconstruction. Repository fixture code is data and is never executed.

The HTTP profile workflow is model-backed but is **not Codex-native**. A result may be called “Codex-native” only when the execution environment actually provides fresh Codex verifier and judge agents through the trusted `prepare-blind` / `submit-echo` / `prepare-reconciliation` / `submit-verdict` lifecycle and the retained run artifact establishes that fact. Do not relabel an HTTP run or substitute it when that runtime capability is unavailable.

## Metrics and failures

JSON and terminal reports cover attempted and persisted cases, verdict accuracy, per-class accuracy, verifier and judge schema compliance, good-patch acceptance, bad-patch detection, false positives and false negatives, `UNCERTAIN` rate, important-finding detection, finding-category accuracy, evidence-reference validation failures, model calls, total/average/median latency, reported tokens, and estimated cost when pricing is supported. The JSON report includes the full 4×4 confusion matrix (`SAME`, `PARTIAL`, `DIFFERENT`, `UNCERTAIN`) and per-case outcomes.

Malformed model output is a benchmark failure. Every case is attempted independently and retained, including failures. Reports distinguish forward, verifier, and judge schema failures, evidence-validation failures, orchestration failures, and provider/runtime failures. A later stage that could not run is `not_attempted`, not successful. Verifier and judge schema compliance are successful decodes divided by actual attempts at that stage.

Important-finding detection remains coarse, case-authored text-probe matching. Finding-category accuracy is separate: it compares the expected category directly with the categories in the trusted materialized verdict. Evidence references outside the immutable candidate patch are validation failures. Unknown usage or pricing stays `null` rather than being invented.

Earlier Benchmark v1 code had three reporting defects: the API loop propagated a per-case error and aborted the suite; schema compliance was hard-coded to 100% for produced reports; and category accuracy reused text-probe success rather than inspecting emitted categories. It also asked the judge for the larger persisted `Verdict` instead of the compact production contract. These behaviors made a live report look healthier than the underlying run and must not be used for historical performance claims.

## Integrity rules

Do not special-case fixtures, weaken schemas or evidence validation, tune a prompt on a case and count that case as unbiased, or revise expected labels after examining Flect output. Prompt-development cases need a separately declared development split before claims on a held-out split. Preserve raw reports with date, code revision, suite hash, model identifiers, and configuration.

## Limitations and next experiments

Forty curated cases are reviewable but small, synthetic, and not representative sampling from software work. Per-class values are descriptive only. Mock success says nothing about live-model quality. Confidence is not calibrated; model snapshots, latency, token reporting, and pricing change. Future work should add independently reviewed real patches, a frozen held-out set, inter-rater agreement, repeated runs for variance, ablations, and a clearly defined no-Flect baseline.

RETRACE and other research results are context, not Flect results. Never attribute their benchmark numbers to Flect or advertise Flect benchmark numbers in the README before a retained real run is complete.
