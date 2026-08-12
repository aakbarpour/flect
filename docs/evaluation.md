# Evaluation

Flect's evaluation harness is a reproducible measurement tool, not a source of precomputed marketing claims. The bundled suite contains ten curated repository-level cases: a correct patch, partial implementation, scope creep, constraint violation, semantic workaround, broadened behavior, unrelated behavior removal, missing edge case, unnecessary refactor, and wrong-component change.

Every case in `fixtures/evaluation/cases.json` contains base files, the original task, a structured candidate patch, an intended specification, deterministic mock outputs, the expected broad verdict, and important expected findings. The harness constructs a strict `BlindBundle` without the original task before backward reconstruction.

## Offline evaluation

The default command is deterministic, requires no credential, performs no network request, and spends no API credits:

```console
flect eval
flect --json eval --output target/flect-eval-offline.json
```

Offline results prove that the harness, schemas, metric aggregation, and fixture expectations are reproducible. A perfect offline score is expected because the mock responses are fixture data; it is not evidence of real-model quality.

## Optional model comparison

Copy and review `fixtures/evaluation/profiles.example.toml`, set its credential environment variable outside the file, and opt in explicitly:

```console
export OPENAI_API_KEY=...
flect eval \
  --profiles fixtures/evaluation/profiles.example.toml \
  --allow-paid-api \
  --output target/flect-eval-api.json
```

PowerShell uses `$env:OPENAI_API_KEY = "..."` and backticks for line continuation. Merely supplying a profiles file is insufficient: the command fails unless `--allow-paid-api` is also present. Normal `cargo test --workspace` execution removes the credential and runs only the offline suite.

The example profiles compare:

- `cheap`: `gpt-5.6-luna`, with escalation disabled.
- `cheap-plus-escalation`: Luna with a single Terra fallback for malformed, uncertain, or below-threshold backward/reconciliation output.
- `stronger`: `gpt-5.6-terra`, with escalation disabled.

Model IDs, endpoint, credential environment variable, reasoning effort, timeouts, and thresholds are configuration rather than permanent assumptions. Each report records the complete non-secret profile and the actual model sequence used per case.

## Metrics

Reports include exact broad-verdict agreement, correct-patch acceptance, bad-patch detection, false positives, uncertainty rate, important-finding recall, requests, latency, input/cached/output tokens, and estimated cost when all required usage and known pricing are available. Unknown token counts or pricing remain `null`; Flect does not invent them.

For bad-patch detection, `PARTIAL` and `DIFFERENT` count as detected. `UNCERTAIN` is reported separately and does not count as successful detection. Important findings use case-authored, case-insensitive substring probes across structured negative findings and evidence descriptions; they are a coarse diagnostic, not a semantic score.

## RETRACE research context

Flect's design is inspired by RETRACE, but its bundled ten-case offline suite is not SWE-bench and has no published Flect real-model results. The following values are reported by the RETRACE paper, not measured by Flect:

| Study configuration | Baseline | RETRACE |
| --- | ---: | ---: |
| mini-SWE-agent + GPT-5-mini, SWE-bench Verified (n=500) | 281/500 (56.2%) | 316/500 (63.2%) |
| MiniMax M2.5, SWE-bench Verified (n=500) | 379/500 (75.8%) | 397/500 (79.4%) |
| GPT-5-mini ablation (n=120) | baseline 60/120 (50.0%) | full RETRACE 73/120 (60.8%) |

The paper's ablation also reports 68/120 (56.7%) for each forward-only and backward-only configuration. These figures motivate Flect's forward intent, blind reconstruction, and reconciliation architecture, but they do not establish Flect effectiveness, cost, false-positive rate, or model quality. See [the RETRACE paper](https://arxiv.org/abs/2608.08950).

## Methodology limits

Ten hand-authored cases are sufficient to expose regressions and compare configurations directionally, but not to support population-level effectiveness claims or per-class precision/recall. There is only one case in most classes. Model confidence is an uncalibrated routing signal. Provider behavior, model snapshots, and pricing can change, so reports should be retained with dates and exact configuration.

RETRACE results above are research context and are never presented as Flect measurements. A no-Flect baseline must be defined before any future comparative claim.
