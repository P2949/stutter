Below is the implementation plan I would follow **before collecting any more KCD1 benchmark data**. The goal is to lock down the new profile-explainability work, document it properly, and add one concrete KCD1 profile-plan artifact so the case study can say more than “the profile applied to 117 tasks.”

## Goal

Finish the profile-explainability follow-up so the repo can show:

> The tested affinity profile did not validate, and `stutter` can now explain which profile rules matched which KCD1/Proton threads before applying the profile.

This should cover:

* code hardening for `profile-plan` / `apply-profile --dry-run --explain`;
* tests for matching semantics and CLI output;
* docs for users;
* an updated KCD1 case-study summary;
* committed KCD1 profile-plan text/JSON artifacts.

## Implementation progress

- [x] Phase 1.1: inspected the named profile explanation, matcher, apply, watch, CLI, renderer, and test code paths before editing.
- [x] Phase 1.2/1.3: hardened CLI validation wording and text output semantics for dry-run explanation and first-match-wins output.
- [x] Phase 2.1: strengthened profile explanation unit tests for process_comm capture and task.comm-only matches.
- [x] Phase 2.2: strengthened CLI parser tests for profile-plan defaults, JSON output, highlights, and `--explain requires --dry-run`.
- [x] Phase 2.3: strengthened renderer tests for first-match-wins, process_comm, KCD-like thread names, and pending affinity text.
- [x] Phase 3: updated `docs/TUNING_WORKFLOW.md`, `examples/profiles/README.md`, and `docs/ARTIFACT_SCHEMA.md` with profile-plan preview and artifact guidance.
- [x] Phase 4: updated the KCD1 case-study report to describe profile explainability as an implemented follow-up.
- [x] Phase 5: generated KCD1 profile-plan text, full JSON, and summary JSON artifacts from the live Gamescope tree using the tuned-only profile file so the artifact targets `kcd1-game-on-1-5-7-11-gamescope-on-0-6`.
- [x] Phase 6: passed formatter, targeted profile explanation/CLI/renderer tests, full workspace tests, clippy with warnings denied, fixture-check, CLI help checks, and JSON/artifact checks.

---

# Phase 1: Verify and harden the explainability implementation

The zip already appears to contain the core implementation, but before documenting it as a finished feature, verify the behavior and fill gaps.

## 1.1 Inspect the relevant code paths

Read these files fully before editing:

```text
stutter/src/profiles/explain.rs
stutter/src/profiles/matching.rs
stutter/src/profiles/apply.rs
stutter/src/watch/apply.rs
stutter/src/watch/profile_explain_render.rs
stutter/src/tune/profile_plan.rs
stutter/src/cli/report.rs
stutter/src/cli/app.rs
stutter/src/cli/tests/report/profile_plan_args.rs
stutter/src/profiles/tests/explain.rs
```

Confirm these properties:

* `profile-plan` uses the same matcher as real profile application.
* `apply-profile --dry-run --explain` cannot accidentally apply changes.
* `match_comm` evidence distinguishes:

  * task `comm`;
  * process `process_comm`.
* First-match-wins behavior is visible in output.
* Rules with zero matches generate a useful warning.
* Broad `process_comm` captures are reported clearly.
* `--highlight-comm` matches both thread `comm` and process `process_comm`.
* JSON output is stable and includes enough semantic fields for report artifacts.
* Text output is readable enough for a human reviewer.

## 1.2 Confirm CLI behavior

Expected commands:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile <profiles.toml>

stutter profile-plan \
  --tree-pid <PID> \
  --profile <profiles.toml> \
  --json \
  --output profile-plan.json

stutter apply-profile \
  --tree-pid <PID> \
  --profile <profiles.toml> \
  --dry-run \
  --explain

stutter apply-profile \
  --tree-pid <PID> \
  --profile <profiles.toml> \
  --dry-run \
  --explain \
  --json \
  --output profile-plan.json
```

Check that invalid combinations fail clearly. For example, decide whether this should be allowed:

```bash
stutter apply-profile --tree-pid <PID> --profile profiles.toml --explain
```

My recommendation: `--explain` should require `--dry-run`, because explanation is for previewing what would happen before applying.

If that is not currently enforced, add a parser or runtime validation error:

```text
--explain requires --dry-run
```

## 1.3 Improve text output if needed

The text output should clearly show:

```text
Profile: kcd1-game-on-1-5-7-11-gamescope-on-0-6
Tree PID: 12345
Snapshot tasks: 184
Matched tasks: 117
Pending affinity changes: 117

Rule 0
  Actions:
    affinity -> 1-5,7-11
  Match:
    match_comm = ["Main"]
  Matched tasks: 104
  Pending affinity: 104
  Classes:
    Helper: 92
    Input: 3
    Recorder: 9
  Top thread comms:
    RenderThread: 1
    ClothingRaycast: 4
    Streaming File: 3
    dxvk-submit: 1
  Top process comms:
    Main: 104
  Broad process_comm captures:
    RenderThread
    ClothingRaycast
    Streaming Async
    dxvk-submit
    dxvk-cs
```

The important report-relevant sentence should be visible in some form:

```text
Rule 0 captured tasks through process_comm = "Main" while their own task comm differed.
```

That directly answers the original ambiguity.

---

# Phase 2: Add or strengthen tests

Do not rely only on live KCD1 output. Add unit and CLI tests so the behavior stays stable.

## 2.1 Profile explanation unit tests

Extend or add tests in:

```text
stutter/src/profiles/tests/explain.rs
```

Add test cases for:

### Test: process_comm capture is reported

Create a fake snapshot with:

```text
task.comm = "RenderThread"
task.process_comm = "Main"
class = Helper
```

Profile:

```toml
[[profile]]
name = "test"

[[profile.rules]]
affinity = "1-5"
match_comm = ["Main"]
```

Expected:

* task is matched;
* match basis includes `process_comm`;
* `broad_process_comm_captured_thread_comms` includes `RenderThread`.

### Test: task_comm match is distinguished from process_comm match

Fake task:

```text
task.comm = "RenderThread"
task.process_comm = "Main"
```

Profile:

```toml
match_comm = ["RenderThread"]
```

Expected:

* match basis includes `task.comm`;
* not counted as broad process capture.

### Test: first-match-wins is visible

Profile:

```toml
[[profile.rules]]
affinity = "1-5"
match_comm = ["Main"]

[[profile.rules]]
affinity = "2-5"
match_comm = ["RenderThread"]
```

Fake task:

```text
task.comm = "RenderThread"
task.process_comm = "Main"
```

Expected:

* task is assigned to rule 0;
* rule 1 does not get the task;
* warning or rule summary makes this understandable.

### Test: zero-match rule warning

Profile has a later rule that never matches. Expected warning:

```text
Rule N matched no tasks
```

or equivalent.

### Test: highlight-comm includes expected task entries

Use `highlight_comm = ["RenderThread"]`.

Expected:

* highlighted task appears in the detailed task section even if it is not in the top-N list.

## 2.2 CLI parser tests

Extend:

```text
stutter/src/cli/tests/report/profile_plan_args.rs
```

Test:

```bash
stutter profile-plan --tree-pid 123 --profile profiles.toml
```

Expected:

* command parses as `ProfilePlan`;
* tree PID is `123`;
* profile path is correct;
* default `top` is correct.

Test JSON/output:

```bash
stutter profile-plan \
  --tree-pid 123 \
  --profile profiles.toml \
  --json \
  --output /tmp/profile-plan.json \
  --top 20 \
  --highlight-comm RenderThread
```

Expected:

* `json = true`;
* output path is parsed;
* top is `20`;
* highlight list contains `RenderThread`.

Also test:

```bash
stutter apply-profile --dry-run --explain --json --output /tmp/explain.json
```

Expected parser fields are correct.

## 2.3 Renderer tests

Extend:

```text
stutter/src/watch/profile_explain_render.rs
```

Add tests that rendered text contains:

```text
first-match-wins
process_comm
RenderThread
ClothingRaycast
pending affinity
```

or whatever exact wording the renderer uses.

The point is to prevent future refactors from stripping out the exact semantic clues the case study depends on.

---

# Phase 3: Update project documentation

This is separate from the KCD1 report. The feature should be discoverable by users who are not reading the case study.

## 3.1 Update `docs/TUNING_WORKFLOW.md`

Add a new section after “Canonical Flow” and before “Verdicts”:

````md
## Preview profile matching before tuning

Before running a tuning experiment, inspect how each profile rule would match the live process tree:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --top 20 \
  --highlight-comm RenderThread \
  --highlight-comm dxvk-submit
````

For apply-profile dry runs, the same explanation can be emitted with:

```bash
stutter apply-profile \
  --tree-pid <PID> \
  --profile profiles.toml \
  --dry-run \
  --explain
```

Use JSON output for artifacts:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --json \
  --output profile-plan.json
```

Profile rules are first-match-wins. `match_comm` checks both a thread's `comm` and its process `process_comm`, so a broad rule such as `match_comm = ["Main"]` may match worker threads whose own thread names are different.

````

Then add a short note in the canonical flow:

```bash
stutter profile-plan --tree-pid <PID> --profile profiles.toml
````

before `stutter tune`.

## 3.2 Update `examples/profiles/README.md`

Add a short “Inspect before tuning” section:

````md
## Inspect before tuning

Profile files are hypotheses. Before running a benchmark, inspect which tasks each rule would match:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile examples/profiles/common-game-layouts.toml \
  --top 20
````

For complex Proton/Wine games, use `--highlight-comm` for important threads:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --highlight-comm RenderThread \
  --highlight-comm dxvk-submit \
  --highlight-comm wineserver
```

This helps catch broad first-match-wins rules and `process_comm` matches before collecting expensive A/B data.

````

## 3.3 Optional: update `docs/ARTIFACT_SCHEMA.md`

If you intend to commit `profile-plan.json` artifacts long-term, add a small schema note:

```md
## Profile plan explanation artifacts

`stutter profile-plan --json` emits a profile explanation artifact containing:

- selected profile name;
- tree PID;
- snapshot task count;
- matched/unmatched task counts;
- per-rule matched task counts;
- actions;
- match criteria;
- match basis;
- class counts;
- top thread `comm`;
- top process `process_comm`;
- broad process-comm captures;
- highlighted task entries;
- warnings.

These artifacts are intended for auditability. They explain what a profile would do before applying or benchmarking it.
````

This is optional, but useful if the JSON will be treated as a first-class artifact.

---

# Phase 4: Update KCD1 case-study report

Update:

```text
reports/kcd1-case-study/CASE_STUDY_SUMMARY.md
```

The current report still says the next priority is profile explainability. That is now stale.

## 4.1 Update key takeaways

Replace:

```md
- The next engineering priority is profile explainability: report which rule matched which tasks, classes, `comm`, `process_comm`, and CPU masks.
```

with:

```md
- A follow-up profile-explainability implementation now allows `stutter` to report which profile rules match which tasks, classes, `comm`, `process_comm`, match source, and proposed CPU masks before a profile is applied.
```

## 4.2 Add a “Profile explainability follow-up” section

Add after “Profile matching caveats”:

````md
## Profile explainability follow-up

After the initial KCD1 A/B experiment, `stutter` gained profile explainability output through:

```bash
stutter profile-plan --tree-pid <PID> --profile <profiles.toml>
stutter apply-profile --tree-pid <PID> --profile <profiles.toml> --dry-run --explain
````

This reports rule-level matched task counts, pending affinity changes, top task `comm`, top `process_comm`, matched classes, match source (`task.comm`, `process_comm`, class, or catch-all), and broad `process_comm` captures.

For this case study, that matters because the KCD process appears as `process_comm = "Main"`, while important worker threads use task names such as `RenderThread`, `ClothingRaycast`, `Streaming Async`, `AudioThread`, and `dxvk-submit`. The explainability output can now show whether those worker threads were matched by the broad `match_comm = ["Main"]` rule, rather than leaving that interpretation implicit.

````

## 4.3 Update current conclusion

Replace:

```md
The next priority, profile explainability, will make the tool's decisions easier to audit and reproduce.
````

with:

```md
The profile explainability follow-up makes this kind of tuning result easier to audit and reproduce by showing which rules matched which tasks before any profile is applied.
```

Also update item 8 in the conclusion list from “next priority” to “implemented follow-up”:

```md
8. A follow-up profile-explainability feature now reports rule-level task matches, classes, `comm`, `process_comm`, and proposed masks so future tuning hypotheses are easier to audit before collecting A/B data.
```

---

# Phase 5: Generate real KCD1 profile-plan artifacts

This is not a new performance data collection step. It is an audit artifact for the existing profile.

You need KCD1 running so `profile-plan` can inspect the live process tree. Use the same stripped-down launch configuration.

## 5.1 Launch KCD1 and detect the process tree

```bash
cd /home/p2949/Desktop/stutter

find_kcd_tree_pid() {
  local pid

  for pid in $(pgrep -f 'gamescope|gamescope-wl' | sort -n); do
    if ./target/release/stutter inspect-tree --tree-pid "$pid" 2>/dev/null \
      | grep -Eq 'RenderThread|dxvk-submit|dxvk-cs|Kingdom:disk|Streaming File|wineserver'; then
      echo "$pid"
      return 0
    fi
  done

  echo "No KCD Gamescope tree found" >&2
  return 1
}

export KCD_TREE_PID="$(find_kcd_tree_pid)"
echo "KCD_TREE_PID=$KCD_TREE_PID"
```

## 5.2 Generate text and JSON profile-plan artifacts

```bash
mkdir -p reports/kcd1-case-study/profiles

./target/release/stutter profile-plan \
  --tree-pid "$KCD_TREE_PID" \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --top 20 \
  --highlight-comm RenderThread \
  --highlight-comm ClothingRaycast \
  --highlight-comm "Streaming Async" \
  --highlight-comm dxvk-submit \
  --highlight-comm dxvk-cs \
  --highlight-comm wineserver \
  --output reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt

./target/release/stutter profile-plan \
  --tree-pid "$KCD_TREE_PID" \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --json \
  --top 20 \
  --highlight-comm RenderThread \
  --highlight-comm ClothingRaycast \
  --highlight-comm "Streaming Async" \
  --highlight-comm dxvk-submit \
  --highlight-comm dxvk-cs \
  --highlight-comm wineserver \
  --output reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json
```

## 5.3 Sanity-check the artifacts

```bash
jq '.' reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json >/dev/null

grep -nE 'RenderThread|ClothingRaycast|Streaming Async|dxvk-submit|dxvk-cs|wineserver|process_comm|first-match|Rule' \
  reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
```

Also extract a small summary from the JSON:

```bash
jq '{
  profile,
  tree_pid,
  snapshot_tasks,
  matched_tasks,
  pending_affinity,
  rules: [
    .rules[] | {
      rule_index,
      actions,
      match_comm,
      match_class,
      matched_tasks,
      pending_affinity,
      classes,
      top_thread_comms,
      top_process_comms,
      broad_process_comm_captured_thread_comms
    }
  ],
  warnings
}' reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json \
  > reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json
```

Commit all three if they are not too large:

```text
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json
```

If the full JSON is too large, commit only the text and summary JSON.

---

# Phase 6: Validation commands

Run these before committing.

## 6.1 Fast targeted checks

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all -- --check

RUSTUP_TOOLCHAIN=nightly cargo test -p stutter profiles::tests::explain
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter cli::tests::report::profile_plan_args
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter watch::profile_explain_render
```

If module paths differ, use broader tests:

```bash
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter profile
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter explain
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter profile_plan
```

## 6.2 Full repo checks

```bash
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-check
```

## 6.3 CLI help checks

```bash
./target/release/stutter profile-plan --help
./target/release/stutter apply-profile --help
```

Confirm help text mentions:

```text
--json
--output
--top
--highlight-comm
--explain
```

## 6.4 Artifact checks

```bash
test -s reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
test -s reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json
jq '.' reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json >/dev/null
```

---

# Phase 7: Commit structure

I would split this into two or three commits.

## Commit 1: code/tests

```text
Explain profile rule matches before applying profiles
```

Include:

```text
stutter/src/profiles/explain.rs
stutter/src/watch/profile_explain_render.rs
stutter/src/watch/apply.rs
stutter/src/tune/profile_plan.rs
stutter/src/cli/report.rs
stutter/src/cli/app.rs
stutter/src/profiles/tests/explain.rs
stutter/src/cli/tests/report/profile_plan_args.rs
```

Only include files actually changed.

## Commit 2: docs/report

```text
Document profile explainability workflow
```

Include:

```text
docs/TUNING_WORKFLOW.md
examples/profiles/README.md
docs/ARTIFACT_SCHEMA.md        # optional
reports/kcd1-case-study/CASE_STUDY_SUMMARY.md
```

## Commit 3: KCD1 audit artifact

```text
Add KCD1 affinity profile-plan artifact
```

Include:

```text
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json
```

If you prefer one commit, that is okay, but separate commits make review easier.

---

# Acceptance criteria

Do not collect more KCD1 benchmark runs until these are true:

* `profile-plan --help` works.
* `apply-profile --dry-run --explain --help` documents the feature.
* JSON output validates with `jq`.
* Text output shows per-rule summaries.
* Output identifies broad `process_comm = "Main"` captures.
* Output can highlight `RenderThread`, `ClothingRaycast`, `Streaming Async`, `dxvk-submit`, `dxvk-cs`, and `wineserver`.
* Case-study summary no longer says profile explainability is merely a future priority.
* `docs/TUNING_WORKFLOW.md` tells users to inspect profile plans before running tune.
* `examples/profiles/README.md` tells users to preview profile matches before using example masks.
* `cargo fmt`, tests, clippy, and fixture-check pass.

Once those are done, the next data collection step can be much smarter: you will be able to inspect a profile’s actual thread placement before spending time on another noisy A/B run.
