# Security policy

Flect handles source code and can send selected code to configured remote model providers. Privacy-boundary failures and accidental disclosure are security issues.

## Reporting a vulnerability

Please do not open a public issue for a suspected secret disclosure, BlindGuard bypass, path-filter escape, command-injection issue, or unsafe state mutation. Use GitHub's private security-advisory flow for the repository. Include affected versions, reproduction steps, impact, and any proposed mitigation.

No response-time guarantee is published. Maintainers will acknowledge valid reports as capacity allows and coordinate disclosure after a fix is available.

## Current boundary

Flect supports deterministic local execution, Responses-compatible API execution, and Codex-native handoffs exposed through the CLI, Skill, and stdio MCP adapter. Git capture is read-only, and Git commands use argument arrays rather than a shell. Flect filters known secret paths and binaries before producing a `BlindBundle`, but it cannot identify every secret embedded in an otherwise ordinary source file. Review `flect inspect` output before sending repository material to a remote provider.

BlindGuard provides a typed structural separation: a strict backward-verifier request has no field for the original task, forward specification, conversation, task-bearing Git metadata, or primary-agent reasoning. This is not cryptographic or operating-system isolation. Repository content can reveal intent, and a reasoner in a shared runtime may be able to inspect resources outside its prepared workspace.

Provider credentials are read from the configured environment variable. Flect does not persist credential values or complete provider payloads and does not intentionally log authorization headers.
