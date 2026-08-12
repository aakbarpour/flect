# Research and attribution

Flect is inspired by:

> **Independent Patch Verification for Coding Agents with a Bidirectional Reconstruct-and-Verify Framework**  
> Chenglin Li, Yisen Xu, Zehao Wang, Shin Hwei Tan, and Tse-Hsun (Peter) Chen  
> arXiv:2608.08950

The RETRACE work motivates the bidirectional reconstruct-and-verify pattern: derive intended behavior in the forward direction, independently reconstruct apparent intent from implementation evidence, then compare the two. Flect does not claim to have invented this academic method, and it does not claim the paper's empirical results as product results.

Flect-specific engineering includes the native Rust CLI, installed-Git workflow, typed on-disk formats, BlindGuard boundary and disclosure report, privacy filtering, provider-neutral runner interface, cost-aware configuration direction, Codex Skill integration plan, machine-readable reporting, and a product-specific evaluation framework. Those decisions productize and extend the pattern for local developer workflows; they are not claims of new academic methodology.
