# Project governance

Flect uses `main` as its canonical integration branch. The deterministic foundation was bootstrapped onto `main`; all subsequent substantive work enters through reviewed pull requests.

## Required lifecycle

```text
Issue
  ↓
Branch from main
  ↓
Cohesive commits
  ↓
Pull request to main
  ↓
CI and review
  ↓
Squash merge
  ↓
Issue closes and branch is deleted
```

Issue descriptions should state the problem, goal, scope, non-goals, acceptance criteria, technical considerations, and validation requirements. Pull requests normally close one primary issue and may reference related work.

## Recommended GitHub settings

The repository is configured for squash merging and automatic branch deletion. Protect `main` with:

- pull requests required before merging;
- the Ubuntu, macOS, and Windows CI matrix required;
- conversation resolution required;
- force pushes disabled;
- branch deletion disabled;
- administrator bypass limited to emergencies.

When repository-plan or permission limitations prevent enforcement, these remain mandatory project policy and should be enabled as soon as GitHub permits it.

## Maintainer checklist

Before merging:

- confirm the pull request targets `main` and links its issue;
- review the full diff for unrelated changes and privacy-boundary regressions;
- ensure format, Clippy, tests, and feature-specific checks pass;
- document known limitations rather than hiding them;
- squash merge only after review is complete.

Do not manually close an issue that a pull request will close on merge. Do not merge red CI or unresolved correctness problems.
