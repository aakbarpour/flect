# Blind verification

Blind verification asks an independent verifier to reconstruct the behavioral task apparent in a patch before it learns the original request. Reconciliation happens only after reconstruction.

## Structural boundary

`BlindBundle` can contain only:

- the sanitized patch set;
- selected context files;
- a manifest of included and excluded paths;
- a BlindGuard isolation report.

It has no field for the original task, issue or pull-request text, conversation, forward spec, primary-agent reasoning, branch name, or commit message. The CLI builds the bundle before it constructs `AgentRequest`; a provider therefore receives the serialized bundle rather than a general application context.

BlindGuard fails closed if configuration would retain Git metadata, branch names, or commit messages. Its report classifies known sources as structurally excluded. Patch-text leakage is marked unknown: code, tests, identifiers, or comments can inherently reveal intent, and no semantic filter can honestly promise otherwise.

## Isolation is not cryptography

“Strict” means Flect used its strict structural policy. It does not mean that the bundle is anonymous, that semantic leakage is impossible, or that the provider cannot infer repository identity. The manifest and `flect inspect` make the actual disclosure reviewable.

## Reconciliation language

- `SAME`: no material divergence was detected. It is not a proof of correctness.
- `PARTIAL`: a requirement, constraint, scope boundary, or side effect diverges.
- `DIFFERENT`: the implementation evidence materially points to another objective.
- `UNCERTAIN`: the available evidence does not support a responsible verdict.

The deterministic reconciler uses conservative lexical coverage over typed fields and remains the offline baseline and fixture oracle. API mode uses a separate semantic reconciliation call after blind reconstruction. The judge emits only the compact contract `alignment`, `findings[{kind,text,evidence_ref}]`, and `confidence`. Flect assigns persisted finding IDs and the recommended action, and resolves stable evidence references into trusted file paths, exact hunks, and line ranges. Unknown or fabricated evidence references fail closed and reject materialization; they are not silently removed.
