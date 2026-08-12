# Isolation assurance

- `strict`: the reasoner cannot access parent context or restricted state through the demonstrated runtime boundary.
- `structural`: Flect supplied no restricted data, but the child/runtime filesystem boundary does not prevent discovery elsewhere.
- `soft`: instructions prohibit restricted context, but neither context nor resource isolation is established.
- `unknown`: capability could not be detected or validated.

Flect's prepared local workspace is structural by default: it contains read-only sanitized resources outside the repository, but that alone is not an operating-system sandbox. A no-parent-context spawn improves conversation isolation without proving the child cannot inspect a shared repository. Report the persisted level; do not upgrade it based on prompt wording.

Record model selection as explicit, inherited, or unknown. Only say a model was selected when the actual spawn mechanism accepted that override.
