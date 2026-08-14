# Research and attribution

Flect is inspired by:

> **Independent Patch Verification for Coding Agents with a Bidirectional Reconstruct-and-Verify Framework**  
> Chenglin Li, Yisen Xu, Zehao Wang, Shin Hwei Tan, and Tse-Hsun (Peter) Chen  
> arXiv:2608.08950

The RETRACE work motivates the bidirectional reconstruct-and-verify pattern: derive intended behavior in the forward direction, independently reconstruct apparent intent from implementation evidence, then compare the two. Flect does not claim to have invented this academic method, and it does not claim the paper's empirical results as product results.

Flect v0.1.0 is released as an independent second opinion for AI-written patches. **Tests pass. But did your coding agent build what you actually asked for?** Flect is not a correctness oracle; it provides another evidence-based signal about patch intent.

## Reported RETRACE benchmarks

The paper reports these results on its own SWE-bench Verified studies:

| Agent/model | Baseline | RETRACE | Change |
| --- | ---: | ---: | ---: |
| mini-SWE-agent + GPT-5-mini (n=500) | 281/500 (56.2%) | 316/500 (63.2%) | +35 issues / +7.0 points |
| MiniMax M2.5 (n=500) | 379/500 (75.8%) | 397/500 (79.4%) | +18 issues / +3.6 points |

For a GPT-5-mini ablation over 120 issues, the paper reports 60/120 (50.0%) baseline, 68/120 (56.7%) forward-only, 68/120 (56.7%) backward-only, and 73/120 (60.8%) full RETRACE.

These are RETRACE results on the authors' dataset and agent configurations. They are neither Flect results nor evidence that Flect produces the same improvement. Flect makes no model-performance claim or correctness guarantee based on this research.

Flect-specific engineering includes the native Rust CLI, installed-Git workflow, typed on-disk formats, BlindGuard boundary and disclosure report, privacy filtering, provider-neutral runner interface, cost-aware configuration, implemented Codex repository plugin, bundled Codex Skill, stdio MCP adapter, machine-readable reporting, and a product-specific evaluation framework. Those decisions productize and extend the pattern for local developer workflows; they are not claims of new academic methodology.
