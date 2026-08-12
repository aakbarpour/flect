# Security policy

Flect handles source code and may eventually send selected code to remote model providers. Privacy-boundary failures and accidental disclosure are security issues.

## Reporting a vulnerability

Please do not open a public issue for a suspected secret disclosure, BlindGuard bypass, path-filter escape, command-injection issue, or unsafe state mutation. Use GitHub's private security-advisory flow for the repository. Include affected versions, reproduction steps, impact, and any proposed mitigation.

No response-time guarantee is published while the project is pre-release. Maintainers will acknowledge valid reports as capacity allows and coordinate disclosure after a fix is available.

## Current boundary

Milestone 1 has no network model provider. Git commands are read-only and execute with argument arrays rather than a shell. Flect filters known secret paths and binaries before producing a `BlindBundle`, but it cannot identify every secret embedded in an otherwise ordinary source file. Always review `flect inspect` output before enabling a future remote provider for a sensitive repository.

