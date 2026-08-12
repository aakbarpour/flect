# Model routing and cost estimates

Flect can run a lower-cost primary model and make at most one fallback request per semantic stage. A typical configuration routes from GPT-5.6 Luna to GPT-5.6 Terra:

```toml
[runner]
kind = "api"
protocol = "responses"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-5.6-luna"
fallback_model = "gpt-5.6-terra"
reasoning_effort = "medium"
timeout_seconds = 120
escalate_on_uncertain = true
confidence_threshold = 0.65
complexity_file_threshold = 12
complexity_byte_threshold = 200000
```

The confidence threshold is an advisory routing heuristic, not a calibrated probability or a quality guarantee. When fallback is configured and escalation is enabled, Flect escalates after:

- an `UNCERTAIN` result or a result containing explicit uncertainties;
- confidence below the configured advisory threshold;
- negative reconciliation findings without structured evidence;
- invalid JSON, schema-invalid output, refusal, incomplete output, or missing output;
- a serialized stage input meeting either configured file-count or byte-count complexity threshold.

A confident primary result invokes no fallback. A fallback result is accepted or the stage fails; Flect never loops or makes more than one fallback attempt for a stage. Machine-readable run and verification records retain every attempted model, its accepted/escalated status, latency, reported input/cached/output tokens, and the escalation reason.

## Versioned pricing table

Cost values are estimates, labeled with pricing version `openai-2026-08-12`, and use text-token list prices per one million tokens:

| Model | Input | Cached input | Output |
| --- | ---: | ---: | ---: |
| `gpt-5.6-luna` | $1.00 | $0.10 | $6.00 |
| `gpt-5.6-terra` | $2.50 | $0.25 | $15.00 |

Flect estimates cost only when the base URL is exactly OpenAI's `https://api.openai.com/v1`, the model ID exactly matches this table, and the provider reports both input and output token counts. Custom endpoints, model aliases, snapshots, missing usage, and unknown models report cost as unavailable. Requests above 272,000 input tokens apply the documented 2× input and 1.5× output multipliers. Estimates exclude non-token charges and cache-write adjustments because the current usage response does not identify cache-write tokens.

Sources: [GPT-5.6 Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna) and [GPT-5.6 Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra).
