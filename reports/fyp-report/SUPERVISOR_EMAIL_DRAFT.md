# Supervisor Email Draft

Subject: FYP supervision request - evidence-based Linux game-performance tuning

Dear Dr [Surname],

I hope you are well. I am currently preparing my Final Year Project idea for Computer Games Development, and I wanted to ask whether the topic might fit your supervision interests.

The project is called “Stutter: Evidence-Based Validation of Linux Game-Performance Tuning”. It focuses on using scheduler-aware eBPF profiling, frame-time data, process-tree tracking, and repeated A/B measurements to evaluate Linux gaming performance tweaks. The aim is not to build a “magic optimizer”, but to create a reproducible workflow that can validate, reject, or mark tuning hypotheses as inconclusive based on evidence.

I already have a working Rust/eBPF prototype and a completed real-world case study using Kingdom Come: Deliverance 1 under Proton/Gamescope. In that case study, a plausible CPU-affinity tuning profile was not validated: the online baseline had a lower primary diagnostic score in all five paired A/B iterations. I think this is a useful result because it shows the tool can avoid turning plausible but unsupported tuning advice into a recommendation.

I have frozen the current prototype and preliminary KCD1 case-study state as a supervisor-review release. I am aware that the project could become too broad, so I would like to scope the FYP around CPU-affinity/process-placement validation, profile explainability, counterbalanced A/B testing, and uncertainty-aware reporting, rather than a general Linux game optimizer or broad autotuning platform.

I have attached a short supervisor pitch. I also made a GitHub release snapshot with the longer report and case-study material here:

https://github.com/P2949/stutter/releases/tag/fyp-report-final

Repository:
https://github.com/P2949/stutter/tree/experimental

I would really appreciate your feedback on whether this is appropriately scoped for an FYP, and whether you might be interested in supervising it.

Kind regards,
[Student's name]
