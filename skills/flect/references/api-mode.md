# API-backed mode

API mode performs forward analysis at `flect start`, then blind reconstruction and reconciliation at `flect verify` using the configured Responses-compatible runner.

Before a remote request, run `flect verify --dry-run` and review provider, model, context, included paths, excluded paths, and BlindGuard metadata. Never add credential values to configuration or messages.

Fallback is bounded to one configured model attempt. Confidence is a heuristic, not a calibrated probability. Do not silently switch from agent mode to a paid API request; use existing explicit API configuration and user authorization.
