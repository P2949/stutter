# E9-E12: hardening the owned-study evidence path (2026-11-11)

Code: `xtask/src/owned_study/{journal,workspace,host,run,manifest,analyze,diagnostics}.rs`; tests `tests{,_journal,_workspace,_host,_diag}.rs`.
Findings from PLAN.md were confirmed in the b666fbab source before any change: `run.rs` used `unwrap_or_default()` on the results read,
`pending_slots` and `ensure_same_context` used `filter_map(...ok())` (a garbled line vanished and its slot could rerun), the append used
`flush()` only, `cold_build_directory` removed `<working_directory>/target` of whatever project the identity named, and
`competing_game` treated every unreadable entry as "no game". All four were real defects.

## E9: strict, crash-aware records and resume
- One verification, `journal::verify_records`, serves resume and analysis. Every record must parse (unknown fields refused), match the study id and
  manifest digest, name a known slot with the scheduled block and arm, be unique, and the recorded slots must be a contiguous SCHEDULE PREFIX
  (out-of-order and gaps refused); validity fields must be consistent; contexts must be one common value, or all absent (legacy).
  Resume additionally requires every record to carry exactly the current context.
- New studies commit one file per slot: a `.started` marker is created and synced before any process launches; the record
  (with its sha256) is written to a temporary file, `sync_all`'d, renamed, and the directory synced. A started-but-uncommitted slot, a leftover
  temporary file, a corrupted or edited record, or an unknown file is an error (`UnsafeToResume`): only a new study id may run that work again.
- `pending_slots` and `ensure_same_context` no longer exist as lenient helpers; a failed verification returns before any `prepare`, marker
  or launch (asserted with a counting fake runner). Existing results are never mutated; the frozen manifest digest is checked.
- Legacy `results.jsonl` studies (E2/E3/E4, E5 context-check) stay readable by the same strict reader, are labelled
  `LegacyNoContext` with the count of context-free records and `no production promotion`, and are never resumed. Re-analysis of E3, E4 and the
  E5 context check left their raw bytes unchanged and reproduced the recorded verdict classes and means exactly.

## E10: owned workspace, no deletion of user build outputs
- Builds run with `CARGO_TARGET_DIR=<study>/workspace/target`. The study root and `workspace/` are `0700`; the workspace carries an
  ownership marker made with `create_new` (study id, manifest digest, uid) that must match to reuse or reset it.
- The source tree is only read. It must be reached without symlink aliases, sit outside the study directory, be a git checkout whose
  `HEAD^{tree}` equals the identity's `git:<tree>` and have no modified tracked files, checked before the study and after every slot
  (a build that modified it invalidates the slot and blocks the next). The tree hash is bound into the context digest.
- Cleanup removes only `workspace/target` after re-checking ownership, that it is a plain directory (not a symlink) and directly inside the
  workspace. Anything else is `UnsafeStudyWorkspace` before a hog or build starts.

## E11: fail-closed host preflight
- `host::inspect` returns Safe, Busy or Incomplete. An unreadable `/proc`, an entry or file it may not read, or an over-large table is Incomplete,
  never "no game". A vanished process or a PID reused between two reads (field 22 start time compared) is skipped. A shell, grep or xtask
  that merely mentions a game is not a game; a genuine gamescope/Steam/Proton/pressure-vessel is Busy; Busy outranks Incomplete.
- Checked before every slot (a non-Safe result stops the study before any marker), after setup and before the compile (hogs cancelled), and
  after the compile (the slot is invalid). The production-smoke preflight, the worker fence and the 40% host-busy gate are untouched.

## E12: method checks
Fewer than 3 independent paired blocks give `Inconclusive` / `InsufficientIndependentBlocks` with no interval; an illegal candidate name is a
verification error, not a ranked row; duplicate, missing-baseline and invalid slots produce no effect row; no failed slot is resampled or
renormalised. The four E3/E4 verdict classes remain as recorded. New optional manifest fields (`output_artifact`, `diagnostics`) are omitted from the
digest when unused, so every old manifest digest is unchanged (asserted).

## Tests (xtask owned_study: 51, all passing; whole xtask suite 620 passing; workspace clippy clean)
PLAN.md required negatives and where they are covered: malformed middle line, truncated final line, blank line, duplicate and unknown field in
`legacy_jsonl_damage_is_never_skipped`; permission denied is not an empty study; duplicate slot, unknown slot, mismatched manifest/study/arm/block and
out-of-order in `mismatched_*`; mixed/missing/changed contexts in `contexts_must_be_present_equal_and_current_when_resuming`; idempotent valid resume and
interrupted resume; simulated death during write (`a_torn_commit_...`, `a_slot_started_but_never_committed_...`); duplicate replay after interrupt; corrupted
record; stale source (`a_source_that_is_not_the_frozen_clean_tree_is_refused`, context binding); existing target with a sentinel unchanged; symlinked
target/parent; path alias; mismatched ownership; interrupted cleanup (idempotent reset); host: unreadable root, denied/broken entry, vanished PID,
same-name shell mention, active gamescope/Steam/Proton, dynamically changing state, valid owned process; role readback mismatch (`readback_must_match_the_request`
and an Invalid slot in `an_invalid_slot_and_a_harness_error_are_recorded_not_repeated`).

## Live drills (this host, 2026-11-11, no game running)
- `kill9-drill/`: a real study was started, its harness process was SIGKILLed mid-slot 2 (slot 1 had committed). One second later there were 0
  surviving stress-ng, fixed-work or compile processes and no `stutter-study` scope. Resuming refused with `slot b01-explicit-all-cpus was started
  but never committed (UnsafeToResume)` and launched nothing.
- `clean-run/`: a clean 4-slot study under the new design, all valid; a resume repeated 0 slots. Builds went to the study-owned workspace
  (65,876 KB there) while the source tree's own `target` stayed at 65,876 KB with an identical listing before and after (`UNCHANGED`).
