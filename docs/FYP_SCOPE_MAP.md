# FYP Scope Map

This map separates the existing `stutter` prototype surface from the proposed
assessed Final Year Project scope. It is intended to reduce first-review noise:
the repository already contains broader systems work, but the proposed FYP is a
bounded validation-methodology project.

| Area | Exists in prototype? | Proposed FYP core? | First-review interpretation |
| --- | --- | --- | --- |
| CPU affinity/process placement | Yes | Yes | Primary tuning area to evaluate. |
| Profile explainability | Yes | Yes | Required pre-tuning audit step. |
| Counterbalanced A/B protocol | Planned | Yes | Core methodology correction motivated by the fixed-order preliminary KCD1 run. |
| Uncertainty-aware reporting | Yes / to refine | Yes | Core reporting and decision vocabulary. |
| Conservative recommendation verdicts | Yes | Yes | Avoids turning weak evidence into advice. |
| KCD1 preliminary case study | Yes | Supporting evidence | Motivation and feasibility evidence, not final proof. |
| Daemon/service paths | Yes | No | Background prototype area unless explicitly rescoped. |
| Remote agent | Yes | No | Out of first-contact FYP scope. |
| Broad autotune controller | Yes | No | Background/future work, not assessed core. |
| GPU power tuning | Yes | No | Out of core scope because it broadens risk and hardware dependence. |
| IRQ affinity | Yes | No | Out of core scope unless a supervisor explicitly narrows to it. |
| VM/kernel tuning | Yes | No | Out of core scope; too broad and system-specific for first FYP framing. |
| Scheduler replacement / SCX | Partial / contextual | No | Mention only as context, not as a deliverable. |
| Packaging/release engineering | Partial | No | Useful hygiene, not proposed assessment substance. |
| Wide hardware/game benchmark suite | No | No | Optional future work; the FYP should prioritize one primary workload. |

## Practical first-contact boundary

A supervisor should be able to judge the proposed FYP by reading the pitch,
`SUPERVISOR_README.md`, `reports/fyp-report/PRE_FYP_SCOPE_NOTE.md`, and the AI
use disclosure. The full source tree and raw KCD1 archive are for audit and
reproduction rather than first-pass supervision triage.
