# Privacy and secret handling

Source code can be sensitive. Flect applies exclusions before `BlindBundle` exists, and the bundle manifest records every selected or rejected path.

Default exclusions cover `.env` variants, PEM/key files, common SSH private-key names, credential and secret filenames, Git internals, binaries, `target`, `dist`, `node_modules`, and `vendor`. Project-specific glob patterns can be added under `[ignore]`. Git's ignore rules govern untracked discovery through `git ls-files --others --exclude-standard`.

Focused context includes non-deleted changed files and a small fixed set of root manifests when present. Per-file, total-context, and total-patch byte limits prevent accidental large disclosures. Patch-only mode adds no file contents beyond diff text. Repository mode is rejected in Milestone 1 rather than silently broadening access.

## What Flect does not promise

Path filtering cannot detect all embedded credentials, personal data, proprietary text, or secrets stored under ordinary filenames. Diff hunks can also contain removed secrets. Review `flect inspect --json` before using a future remote runner, add project-specific ignore patterns, and keep provider data policies in mind.

Flect does not store provider credentials or print complete provider payloads in normal logs. The local `.flect/` run record does store the original task and should remain ignored and access-controlled like other local development metadata.

