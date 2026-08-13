# Getting started

Flect has one trusted Rust application layer and three entry points: direct CLI/API use, a Codex Skill, and a structured MCP server. API mode delegates semantic stages to the configured Responses-compatible endpoint. Codex-native mode asks the active runtime to spawn a fresh verifier and a different fresh judge, while Flect remains authoritative for sanitization, schemas, lifecycle, evidence validation, and persistence.

## Direct API mode

Initialize a Git repository and configure the Responses-compatible runner without putting a credential in TOML:

```console
flect init
# Defaults: gpt-5.6-luna primary, with one gpt-5.6-terra fallback.
# Override either model only when your project needs a different route.
flect config set runner.kind api
flect config show
```

Set `OPENAI_API_KEY` in the process environment. Custom providers can set `runner.base_url`, `runner.api_key_env`, and arbitrary model IDs with the same command. Capture intent before implementation:

```console
flect start --task "Fix token expiry without changing legacy auth"
# implementation and the repository's normal tests happen here
flect verify --dry-run
flect verify
```

`gpt-5.6-luna` is the default primary and `gpt-5.6-terra` is the default bounded fallback; both remain configurable. Flect tries the primary once and can fall back once for malformed output, low confidence, uncertainty, or configured complexity signals. Use `runner.fallback_model none` to clear the fallback. See [model routing](model-routing.md).

## Codex Skill mode

From the target repository:

```console
flect init
flect skill install
flect skill status
```

Then ask Codex: `Use Flect verification for this implementation.` When a collaboration runtime supports a child with no inherited conversation, the Skill prepares a blind job and calls that runtime capability with only the returned read-only resources. A separate fresh child reconciles the accepted echo. Model overrides are optional and must be recorded as explicit, inherited, or unknown; do not claim a particular model unless the runtime accepted it.

The local workspace boundary is `structural`, not an OS sandbox. The child receives no task or parent turns, but a shared runtime may still permit access outside the prepared workspace. If fresh subagents are unavailable, deliberately use configured API mode or report that independent agent verification is unavailable.

## MCP mode

Register `flect mcp` using the current Codex stdio configuration in [the MCP guide](mcp.md). A Codex-native tool sequence is:

1. `flect_start` with the exact user task before edits.
2. Run implementation and repository tests.
3. `flect_prepare_blind`, then spawn a fresh no-parent-context verifier with only that job's allowed resources.
4. `flect_submit_echo` with the verifier's typed response.
5. `flect_prepare_reconciliation`, then spawn a different fresh judge with only that contract.
6. Have the judge write the exact generated `ReconciliationAgentSubmission` file and stop. The trusted orchestrator then calls `flect_submit_verdict` or `flect agent submit-verdict --submission-file <submission_file>` using only that opaque designated path; never parse or relay the file or judge chat response.
7. `flect_get_result` if the result is needed later.

For automated API mode, use `flect_inspect` followed by `flect_verify`. Both modes use the same domain types, evidence policy, and local `.flect/` store.

## Privacy checks

Run `flect inspect` or `flect verify --dry-run` before a remote request. Dry-run reports provider/model selection, context policy, included files, and excluded files, and does not read the API key or initialize a remote runner. Default filtering excludes common secrets, Git internals, build/dependency output, and binary content. Review [privacy](privacy.md) for limitations: source code and comments can reveal task semantics, and heuristic secret filtering cannot prove that arbitrary proprietary content is safe to transmit.

## Diagnostics and troubleshooting

`flect doctor` checks Git, repository/config state, API credential readiness, Codex CLI availability, the project-local Skill, and MCP readiness. It reports agent spawning, no-parent-context support, and model overrides as `unknown` because a standalone binary cannot inspect the active Codex collaboration tool surface. It prints only the credential environment variable name, never its value.

- `API credential ... missing`: set the configured environment variable in the shell that launches Flect.
- `unsupported request`: the configured endpoint likely lacks the required Responses structured-output behavior; change provider/model parameters or endpoint.
- `verification result ... does not exist`: run `flect verify` for that run before `flect_get_result`.
- Skill `modified`: preserve or remove the user-edited files explicitly; Flect refuses to overwrite them.
- `UNCERTAIN`: do not report successful verification. Review context exclusions, raise context deliberately, or configure a supported fallback.

## Known limitations

- The Responses-style protocol is implemented; Chat Completions compatibility is not yet implemented.
- Codex-native mode establishes fresh conversation handoff only when the runtime actually supports it; Flect reports its local resource boundary as structural.
- A configured API-backed backward request is `strict` with respect to parent conversation access, subject to the provider and source-content caveats in the privacy model.
- The ten-case evaluation suite supports regression checks and directional comparison, not broad effectiveness claims.
- No real-model benchmark result or first public release has been published yet.
- Native archives exist as a release workflow; Homebrew, Scoop, and other feeds are intentionally deferred.
