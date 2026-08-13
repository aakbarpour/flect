# Privacy and secret handling

Source code can be sensitive. Flect applies exclusions before `BlindBundle` exists, and the bundle manifest records every selected or rejected path.

Default exclusions cover `.env` variants, PEM/key files, common SSH private-key names, credential and secret filenames, Git internals, binaries, `target`, `dist`, `node_modules`, and `vendor`. Project-specific glob patterns can be added under `[ignore]`. Git's ignore rules govern untracked discovery through `git ls-files --others --exclude-standard`. Untracked capture accepts only regular files within the repository boundary; symbolic links and special filesystem objects are not dereferenced into a bundle.

Focused context includes non-deleted changed files and a small fixed set of root manifests when present. Per-file, total-context, and total-patch byte limits bound capture. Patch-only mode adds no file contents beyond diff text. Repository context remains rejected rather than silently broadening access.

Responses-compatible API mode sends the selected typed request to the configured endpoint. Codex-native mode writes sanitized resources to a temporary, read-only workspace for fresh verifier and judge handoffs. CLI, Skill, and MCP entry points use the same application operations and privacy policy. Inspect the dry run or `flect inspect --json`, including its manifest, before enabling remote execution for a sensitive repository.

## What Flect does not promise

Path filtering cannot detect all embedded credentials, personal data, proprietary text, or secrets stored under ordinary filenames. Diff hunks can also contain removed secrets. Structural isolation is not anonymity, cryptographic isolation, or an operating-system sandbox. Code, comments, paths, and identifiers may reveal the task or repository identity, and a child process or agent sharing the host may have access beyond the prepared workspace.

Flect does not store provider credentials or print complete provider payloads in normal logs. Project-local `.flect/` state does store the original task, selected bundle, results, and safe call metadata; keep it ignored and protect it like other sensitive development metadata.
