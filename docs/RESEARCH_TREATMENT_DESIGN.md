# Within-source treatment design (implemented offline; not exercised live)

Status: implemented (offline, fixture-verified) in the commit that introduced `treatment.rs`: the
production funnel no longer blocks feature and action studies. The proposal below is what was built;
the real-host and live-game evidence it needs does not exist yet.

## The defect

`StudyArms::Pair` makes every non-screen study a comparison of two **Proton builds**. A feature study
(one factor off versus on) and an action study (Stutter action versus no action) compare two
**treatments of one build**. `ProtonSource` is both the arm key and the source identity throughout the
schedule (`AttemptSlot.arm`), the dispatch context, native evidence, the report (control/candidate) and
the verifiers. Enabling a factor "for both arms" cannot isolate its effect, and the selected control build
may not support it at all.

## Required shape

1. An arm becomes `(source, treatment)` where `treatment` is `None` for every existing study. Serialization
   skips `None`, so every existing intent, journal, evidence file and report keeps its bytes and digest
   (the `child_study_sha256` / `confirmation_rule` precedent).
2. A treatment study's intent carries exactly one tool and a frozen `treatment` pair (off, on) naming one
   authorized factor or action. `child_intent` derives it; `child_verify::derived_from` re-derives it from
   the exact planned study (as it now does for arms, budget and rule).
3. The schedule balances the two treatments of that one build in complete seeded blocks
   (`scheduler::balanced` over two virtual arms); dispatch applies the factor only to the "on" arm; the
   action gate is armed only for "on" attempts, and "off" attempts must show no held action.
4. Analysis, report and `proton-comparison.json` name the treatment, never a second Proton.
5. Qualification is three separate predicates: Proton comparison (existing), factor effective activation
   (runtime evidence, not an environment variable), and a real Supported action (kernel-state receipts,
   `ActionGate`). A qualified benchmark alone never qualifies a feature or action.
6. Selecting which build to test comes from the qualified screen (the control), not a pair.

## Compatibility constraints

- Old campaigns must keep verifying: the preserved campaign `fdf5ba08` replays today (`real_campaign_replay`).
- No new root IPC and no second optimizer: the existing normal-controller path applies the action.
- Until this exists the studies stay blocked and recorded; they are never run as two-Proton comparisons.
