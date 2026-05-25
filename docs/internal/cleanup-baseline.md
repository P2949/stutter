# Cleanup Baseline

This document captures the baseline of the top 50 largest files before the massive refactoring phases begin.
Each subsequent phase should split these files and document the before/after LOC sizes.

## Top 50 Largest Files

| LOC  | File Path                                              | Target Phase / Split Plan |
|------|--------------------------------------------------------|---------------------------|
| 1510 | ./stutter-ebpf/src/main.rs                             | Phase 5                   |
| 1055 | ./xtask/src/main.rs                                    | Phase 31                  |
| 993  | ./stutter/src/actions/irq_affinity.rs                  | Phase 8                   |
| 981  | ./stutter/src/session/monitor_session.rs               | Phase 15                  |
| 981  | ./stutter/src/cli/mod.rs                               | Phase 18                  |
| 972  | ./stutter/src/session_io.rs                            | Phase 16                  |
| 969  | ./stutter/src/config/merge.rs                          | Phase 4                   |
| 968  | ./stutter/src/recorder/session.rs                      | Phase 17                  |
| 967  | ./stutter/src/autotune/controller/tests.rs             | Phase 37                  |
| 959  | ./stutter/src/cli/report/tests.rs                      | Phase 37                  |
| 957  | ./stutter/src/tui.rs                                   | Phase 19                  |
| 952  | ./stutter/src/metrics.rs                               | Phase 20                  |
| 949  | ./stutter/src/autotune/planner_tests/workload_policy.rs|                           |
| 947  | ./stutter/src/recording_fixture_tests.rs               |                           |
| 937  | ./stutter/src/tune/mod.rs                              |                           |
| 933  | ./stutter/src/watch.rs                                 | Phase 26                  |
| 933  | ./stutter/src/report/render/text.rs                    | Phase 22                  |
| 932  | ./stutter/src/autotune/rolling_window/tests.rs         |                           |
| 924  | ./stutter/src/mangohud.rs                              | Phase 23                  |
| 924  | ./stutter/src/display_path_compare.rs                  | Phase 24                  |
| 914  | ./stutter/src/autotune/objective.rs                    | Phase 10                  |
| 908  | ./stutter/src/agent/tests/autotune.rs                  |                           |
| 903  | ./stutter/src/report/analysis/timing.rs                | Phase 21                  |
| 899  | ./stutter/src/autotune/workload_policy.rs              | Phase 13                  |
| 887  | ./stutter/src/autotune/planning/profile_candidates.rs  | Phase 14                  |
| 887  | ./stutter/src/actions/cgroup/tests.rs                  |                           |
| 883  | ./stutter/src/autotune/runtime/tests.rs                |                           |
| 871  | ./stutter/src/autotune/candidate_memory.rs             | Phase 11                  |
| 867  | ./stutter/src/affinity.rs                              | Phase 25                  |
| 860  | ./stutter/src/profiles.rs                              | Phase 25                  |
| 854  | ./stutter/src/autotune/controller_journal.rs           |                           |
| 844  | ./stutter/src/ebpf/tests/loader.rs                     |                           |
| 842  | ./stutter/src/report/tests/foreground_focus.rs         |                           |
| 832  | ./stutter/src/recorder/session_files.rs                |                           |
| 832  | ./stutter/src/doctor.rs                                |                           |
| 830  | ./stutter/src/daemon/runtime.rs                        |                           |
| 825  | ./stutter/src/service.rs                               |                           |
| 823  | ./stutter/src/autotune/runtime.rs                      | Phase 12                  |
| 823  | ./stutter/src/actions/nice.rs                          |                           |
| 810  | ./stutter/src/actions/error.rs                         |                           |
| 809  | ./stutter/src/autotune/providers/irq_affinity.rs       |                           |
| 804  | ./stutter/src/tune/comparability.rs                    |                           |
| 804  | ./stutter/src/focus/tests/foreground.rs                |                           |
| 802  | ./stutter/src/actions/uclamp.rs                        |                           |
| 801  | ./stutter/src/daemon/soak.rs                           |                           |
| 792  | ./stutter/src/recommend.rs                             |                           |
| 786  | ./stutter/src/autotune/startup_recovery/tests.rs       |                           |
| 781  | ./stutter/src/cli/daemon.rs                            |                           |
| 777  | ./stutter/src/autotune/live_experiment/mod.rs          |                           |
