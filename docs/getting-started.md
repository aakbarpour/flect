# Getting started

Flect has one Rust verification pipeline and three entry points: direct CLI/API use, a Codex Skill that teaches the workflow, and an MCP server that exposes structured tools. The Skill and MCP server do not create hidden Codex sessions or select models from a Codex subscription. Strict independent model selection uses Flect's configured API runner.

## Direct API mode

Initialize a Git repository and configure the Responses-compatible runner without putting a credential in TOML:

```console
flect init
flect config set runner.model gpt-5.6-luna
flect config set runner.fallback_model gpt-5.6-terra
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

`gpt-5.6-luna` and `gpt-5.6-terra` are configurable starting candidates, not claims of optimality. Flect tries the primary once and can fall back once for malformed output, low confidence, uncertainty, or configured complexity signals. Use `runner.fallback_model none` to clear the fallback. See [model routing](model-routing.md).

## Codex Skill mode

From the target repository:

```console
flect init
flect skill install
flect skill status
```

Then ask Codex: `Use Flect verification for this implementation.` The Skill directs the active agent to capture the task before coding, run project checks, and then invoke Flect. The Skill is orchestration only; when Flect is configured for API mode, the backward call is independently strict. In mock mode, the output is a deterministic offline baseline. The active Codex agent itself is not blind.

## MCP mode

Register `flect mcp` using the current Codex stdio configuration in [the MCP guide](mcp.md). A normal tool sequence is:

1. `flect_start` with the exact user task before edits.
2. Run implementation and repository tests.
3. `flect_inspect` to review the outbound blind bundle.
4. `flect_verify` to persist a verdict.
5. `flect_get_result` to retrieve it later.

The MCP adapter delegates to the same CLI pipeline and local `.flect/` store. It does not implement a second verifier.

## Privacy checks

Run `flect inspect` or `flect verify --dry-run` before a remote request. Dry-run reports provider/model selection, context policy, included files, and excluded files, and does not read the API key or initialize a remote runner. Default filtering excludes common secrets, Git internals, build/dependency output, and binary content. Review [privacy](privacy.md) for limitations: source code and comments can reveal task semantics, and heuristic secret filtering cannot prove that arbitrary proprietary content is safe to transmit.

## Diagnostics and troubleshooting

`flect doctor` checks Git, repository/config state, runner and credential readiness, Codex CLI availability, the project-local Skill, and MCP command readiness. It prints only the credential environment variable name, never its value.

- `API credential ... missing`: set the configured environment variable in the shell that launches Flect.
- `unsupported request`: the configured endpoint likely lacks the required Responses structured-output behavior; change provider/model parameters or endpoint.
- `verification result ... does not exist`: run `flect verify` for that run before `flect_get_result`.
- Skill `modified`: preserve or remove the user-edited files explicitly; Flect refuses to overwrite them.
- `UNCERTAIN`: do not report successful verification. Review context exclusions, raise context deliberately, or configure a supported fallback.

## Known limitations

- The Responses-style protocol is implemented; Chat Completions compatibility is not yet implemented.
- Codex Skill/MCP orchestration does not imply separate model entitlement or hard session isolation.
- Only configured API-backed backward requests have the structural strict-blind boundary.
- The ten-case evaluation suite supports regression checks and directional comparison, not broad effectiveness claims.
- No real-model benchmark result or first public release has been published yet.
- Native archives exist as a release workflow; Homebrew, Scoop, and other feeds are intentionally deferred.
