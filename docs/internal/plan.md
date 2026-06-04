Here is the implementation plan for the **multi-profile selection fix**. The goal is to make `profile-plan` and `apply-profile --dry-run --explain` reproducible when the TOML contains more than one profile, especially for:

```text
reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml
```

where the first profile is `baseline-online` and the tuned profile is second.

---

# Plan: Add explicit profile selection to `profile-plan` and `apply-profile`

## Goal

Add:

```bash
--profile-name <NAME>
```

to both:

```bash
stutter profile-plan
stutter apply-profile
```

so users can run:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

and reliably inspect the intended profile instead of silently inspecting the first profile in the file.

## Implementation progress

- [x] Phase 1: inspected the current CLI, watch/apply, tune profile-plan, profile parser, explain, matcher, and local CLI parser tests; `stutter/src/cli/tests/app/apply_profile_args.rs` is not present in this repo, so apply-profile parser coverage is in `stutter/src/cli/tests/report/profile_plan_args.rs`.
- [x] Phase 2: added and re-exported `load_selected_profile`, preserving default first-profile behavior and listing available profiles for missing names.
- [x] Phase 3: added `--profile-name <NAME>` to `apply-profile` and `profile-plan`.
- [x] Phase 4: carried `profile_name` through CLI parser inputs, command inputs, and watch command inputs.
- [x] Phase 5: switched single-profile apply/explain commands to `load_selected_profile` without changing `tune` profile loading.
- [x] Phase 6: added `profile_path` and `profile_name_requested` to explanation JSON and rendered them in text output when present.
- [x] Phase 7: added loader tests, CLI parser tests, and a selected-profile explanation regression test for a two-profile TOML.
- [x] Phase 8: updated workflow docs, examples README, artifact schema, and KCD1 report with `--profile-name` guidance for multi-profile TOML files.
- [x] Phase 9: regenerated KCD1 profile-plan text, JSON, and summary artifacts from `kcd1-affinity-ab.toml` with explicit `--profile-name`; artifacts record the selected tuned profile and highlighted KCD/DXVK threads.
- [x] Phase 10: run validation commands.

---

# Phase 1: Inspect current code paths

Before editing, read these files fully:

```text
stutter/src/cli/report.rs
stutter/src/cli/app.rs
stutter/src/tune/profile_plan.rs
stutter/src/watch/apply.rs
stutter/src/profiles/mod.rs
stutter/src/profiles/explain.rs
stutter/src/profiles/matching.rs
stutter/src/cli/tests/report/profile_plan_args.rs
stutter/src/cli/tests/app/apply_profile_args.rs
```

Look specifically for current use of:

```rust
load_first_profile(...)
```

Likely affected call sites:

```rust
crate::profiles::load_first_profile(&input.profile_path)?
```

The fix is to replace those with a selected-profile loader.

---

# Phase 2: Add a shared selected-profile loader

Add a helper in the profiles module, probably in:

```text
stutter/src/profiles/mod.rs
```

or a small new file:

```text
stutter/src/profiles/load.rs
```

Recommended API:

```rust
pub fn load_selected_profile(
    path: &std::path::Path,
    profile_name: Option<&str>,
) -> anyhow::Result<Profile> {
    let profiles = load_profiles(path)?;

    if let Some(name) = profile_name {
        return profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "profile '{}' not found in {}; available profiles: {}",
                    name,
                    path.display(),
                    available_profile_names(path)
                        .unwrap_or_else(|_| "<failed to list profiles>".to_string())
                )
            });
    }

    profiles.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!("no profiles found in {}", path.display())
    })
}
```

Better implementation: avoid re-reading the file for available names. Do it from the same `profiles` vector:

```rust
pub fn load_selected_profile(
    path: &std::path::Path,
    profile_name: Option<&str>,
) -> anyhow::Result<Profile> {
    let profiles = load_profiles(path)?;

    if let Some(name) = profile_name {
        let available = profiles
            .iter()
            .map(|profile| profile.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        return profiles
            .into_iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "profile '{}' not found in {}; available profiles: {}",
                    name,
                    path.display(),
                    if available.is_empty() { "<none>" } else { &available }
                )
            });
    }

    profiles.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!("no profiles found in {}", path.display())
    })
}
```

Keep `load_first_profile` for backwards compatibility if other code still uses it.

---

# Phase 3: Add `--profile-name` to CLI args

## 3.1 `profile-plan`

Find the args struct for `profile-plan`, likely in:

```text
stutter/src/cli/report.rs
```

or equivalent.

Add:

```rust
#[arg(long = "profile-name", value_name = "NAME")]
pub(super) profile_name: Option<String>,
```

The help text should say something like:

```rust
/// Select a named profile from a multi-profile TOML file.
```

Expected help output:

```text
--profile-name <NAME>    Select a named profile from a multi-profile TOML file
```

## 3.2 `apply-profile`

Find `ApplyProfileArgs`, likely in:

```text
stutter/src/cli/app.rs
```

Add the same field:

```rust
#[arg(long = "profile-name", value_name = "NAME")]
pub(super) profile_name: Option<String>,
```

This should work for both normal apply and dry-run explain mode.

---

# Phase 4: Carry the field through command inputs

Add `profile_name` to any command input structs.

Likely structs:

```text
ProfilePlanCommandInput
ApplyProfileCommandInput
watch::ApplyProfileCommandInput
watch::ProfilePlanCommandInput
```

The shape should be:

```rust
pub struct ProfilePlanCommandInput {
    pub tree_pid: u32,
    pub profile_path: PathBuf,
    pub profile_name: Option<String>,
    pub json: bool,
    pub output: Option<PathBuf>,
    pub top: usize,
    pub highlight_comm: Vec<String>,
}
```

For apply profile:

```rust
pub struct ApplyProfileCommandInput {
    pub tree_pid: u32,
    pub profile_path: PathBuf,
    pub profile_name: Option<String>,
    pub dry_run: bool,
    pub explain: bool,
    pub json: bool,
    pub output: Option<PathBuf>,
    // existing fields...
}
```

Make sure all construction sites pass through:

```rust
profile_name: args.profile_name.clone(),
```

or move it directly if owned.

---

# Phase 5: Use `load_selected_profile`

Replace:

```rust
let profile = crate::profiles::load_first_profile(&input.profile_path)?;
```

with:

```rust
let profile = crate::profiles::load_selected_profile(
    &input.profile_path,
    input.profile_name.as_deref(),
)?;
```

Do this in:

```text
stutter/src/tune/profile_plan.rs
stutter/src/watch/apply.rs
```

and any other command path that loads the profile for `apply-profile`.

Important: do **not** change `stutter tune` profile loading semantics. `tune` should still load all profiles from the file. This fix is only for commands that apply or explain **one selected profile**.

---

# Phase 6: Update explain output to show source profile file and selected profile

The existing output already shows the profile name. Add or confirm it shows enough context:

```text
Profile plan: kcd1-game-on-1-5-7-11-gamescope-on-0-6
Profile file: reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml
Selected with: --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

In JSON, add fields if not already present:

```json
{
  "profile": "kcd1-game-on-1-5-7-11-gamescope-on-0-6",
  "profile_path": "reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml",
  "profile_name_requested": "kcd1-game-on-1-5-7-11-gamescope-on-0-6"
}
```

This is optional but useful for reproducibility. At minimum, the selected profile name must be present.

---

# Phase 7: Add tests

## 7.1 CLI parser tests

Add or extend tests in:

```text
stutter/src/cli/tests/report/profile_plan_args.rs
stutter/src/cli/tests/app/apply_profile_args.rs
```

Tests:

### `profile_plan_accepts_profile_name`

Input:

```bash
stutter profile-plan \
  --tree-pid 123 \
  --profile profiles.toml \
  --profile-name tuned
```

Expected:

```rust
profile_name == Some("tuned")
```

### `profile_plan_accepts_profile_name_with_json_output`

Input:

```bash
stutter profile-plan \
  --tree-pid 123 \
  --profile profiles.toml \
  --profile-name tuned \
  --json \
  --output profile-plan.json
```

Expected:

```rust
profile_name == Some("tuned")
json == true
output == Some(...)
```

### `apply_profile_accepts_profile_name`

Input:

```bash
stutter apply-profile \
  --tree-pid 123 \
  --profile profiles.toml \
  --profile-name tuned \
  --dry-run \
  --explain
```

Expected:

```rust
profile_name == Some("tuned")
dry_run == true
explain == true
```

## 7.2 Loader tests

Add tests near profile-loading tests, likely in:

```text
stutter/src/profiles/tests/...
```

Create a temporary TOML with two profiles:

```toml
[[profile]]
name = "baseline-online"

[[profile.rules]]
affinity = "online"

[[profile]]
name = "tuned"

[[profile.rules]]
affinity = "1-5"
match_comm = ["Main"]
```

Tests:

### `load_selected_profile_defaults_to_first_profile`

```rust
let profile = load_selected_profile(path, None)?;
assert_eq!(profile.name, "baseline-online");
```

This preserves old behavior.

### `load_selected_profile_selects_named_profile`

```rust
let profile = load_selected_profile(path, Some("tuned"))?;
assert_eq!(profile.name, "tuned");
```

### `load_selected_profile_rejects_missing_name`

```rust
let err = load_selected_profile(path, Some("missing")).unwrap_err();
assert!(err.to_string().contains("profile 'missing' not found"));
assert!(err.to_string().contains("baseline-online"));
assert!(err.to_string().contains("tuned"));
```

## 7.3 Integration-style explanation test

Create a test that proves `profile-plan` explanation uses the selected profile, not the first one.

Use a profile file:

```toml
[[profile]]
name = "baseline-online"

[[profile.rules]]
affinity = "online"
match_class = ["Game", "Helper"]

[[profile]]
name = "tuned"

[[profile.rules]]
affinity = "1-5"
match_comm = ["Main"]
```

Fake task:

```text
comm = RenderThread
process_comm = Main
class = Helper
current_affinity = 0-11
```

Expected when selecting `tuned`:

```text
profile == tuned
pending_affinity > 0
broad_process_comm_captured_thread_comms includes RenderThread
```

Expected without selecting name:

```text
profile == baseline-online
```

This test protects against the exact KCD1 bug.

---

# Phase 8: Update docs

## 8.1 `docs/TUNING_WORKFLOW.md`

Update every profile-plan example that uses a multi-profile file.

Before:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml
```

After:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

Add a warning:

```md
When a TOML file contains multiple `[[profile]]` entries, commands that inspect or apply a single profile default to the first profile unless `--profile-name` is provided. For A/B files, always pass `--profile-name` when inspecting the tuned profile.
```

## 8.2 `examples/profiles/README.md`

Add the same idea:

````md
For multi-profile files, select the profile explicitly:

```bash
stutter profile-plan \
  --tree-pid <PID> \
  --profile profiles.toml \
  --profile-name tuned-profile-name
````

Without `--profile-name`, single-profile commands use the first profile in the file for backwards compatibility.

````

## 8.3 `docs/ARTIFACT_SCHEMA.md`

Fix the minor formatting issue:

```md
`stutter
profile-plan --json`
````

to:

```md
`stutter profile-plan --json`
```

Also mention:

```md
Profile-plan JSON artifacts should record the selected profile name. When generated from a multi-profile file, the command should include `--profile-name` for reproducibility.
```

## 8.4 `reports/kcd1-case-study/CASE_STUDY_SUMMARY.md`

Update the profile-plan commands in the report so they include:

```bash
--profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

Add one sentence in the profile explainability section:

```md
Because the case-study TOML contains both `baseline-online` and the tuned affinity profile, the profile-plan artifact is generated with `--profile-name` so the audited profile is the tuned profile rather than the first profile in the file.
```

---

# Phase 9: Regenerate KCD1 profile-plan artifacts

After implementing the CLI fix, launch KCD1 and regenerate the artifacts from the real A/B TOML.

## 9.1 Detect tree PID

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

## 9.2 Generate text artifact

```bash
./target/release/stutter profile-plan \
  --tree-pid "$KCD_TREE_PID" \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6 \
  --top 20 \
  --highlight-comm RenderThread \
  --highlight-comm ClothingRaycast \
  --highlight-comm "Streaming Async" \
  --highlight-comm dxvk-submit \
  --highlight-comm dxvk-cs \
  --highlight-comm wineserver \
  --output reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
```

## 9.3 Generate JSON artifact

```bash
./target/release/stutter profile-plan \
  --tree-pid "$KCD_TREE_PID" \
  --profile reports/kcd1-case-study/profiles/kcd1-affinity-ab.toml \
  --profile-name kcd1-game-on-1-5-7-11-gamescope-on-0-6 \
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

## 9.4 Regenerate summary JSON

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

## 9.5 Sanity-check artifacts

```bash
jq '.' reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json >/dev/null

grep -nE 'Profile plan:|profile-name|Rule|RenderThread|ClothingRaycast|Streaming Async|dxvk-submit|dxvk-cs|wineserver|process_comm|first-match' \
  reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
```

Expected:

```text
Profile plan: kcd1-game-on-1-5-7-11-gamescope-on-0-6
Rule 0 ...
RenderThread ...
ClothingRaycast ...
dxvk-submit ...
process_comm ...
```

Also confirm it does **not** say:

```text
Profile plan: baseline-online
```

---

# Phase 10: Validation commands

Run targeted checks first:

```bash
RUSTUP_TOOLCHAIN=nightly cargo fmt --all -- --check

RUSTUP_TOOLCHAIN=nightly cargo test -p stutter load_selected_profile
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter profile_plan
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter apply_profile
RUSTUP_TOOLCHAIN=nightly cargo test -p stutter explain
```

Then full checks:

```bash
RUSTUP_TOOLCHAIN=nightly cargo test --all
RUSTUP_TOOLCHAIN=nightly cargo clippy --all-targets -- -D warnings
RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- fixture-check
```

Check CLI help:

```bash
./target/release/stutter profile-plan --help | grep -A2 -B2 profile-name
./target/release/stutter apply-profile --help | grep -A2 -B2 profile-name
```

---

# Phase 11: Suggested commit structure

## Commit 1

```text
Select named profiles for profile-plan and apply-profile
```

Include:

```text
stutter/src/profiles/...
stutter/src/cli/...
stutter/src/tune/profile_plan.rs
stutter/src/watch/apply.rs
tests
```

## Commit 2

```text
Document named profile selection for profile explainability
```

Include:

```text
docs/TUNING_WORKFLOW.md
examples/profiles/README.md
docs/ARTIFACT_SCHEMA.md
reports/kcd1-case-study/CASE_STUDY_SUMMARY.md
```

## Commit 3

```text
Regenerate KCD1 profile-plan artifacts from A/B profile
```

Include:

```text
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.txt
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan.json
reports/kcd1-case-study/profiles/kcd1-affinity-profile-plan-summary.json
```

---

# Acceptance criteria

The fix is complete only when:

* `profile-plan --profile-name <NAME>` selects the named profile.
* `apply-profile --profile-name <NAME>` selects the named profile.
* Missing profile names produce a clear error listing available profiles.
* Omitting `--profile-name` keeps the old first-profile behavior.
* Tests cover named selection, default selection, and missing-profile error.
* Docs warn that multi-profile TOML files need `--profile-name`.
* KCD1 profile-plan artifacts are regenerated from `kcd1-affinity-ab.toml` with explicit `--profile-name`.
* The regenerated artifact says:

```text
Profile plan: kcd1-game-on-1-5-7-11-gamescope-on-0-6
```

* The artifact shows `RenderThread`, `ClothingRaycast`, `Streaming Async`, `dxvk-submit`, `dxvk-cs`, and `wineserver`.
* `cargo fmt`, tests, clippy, and fixture-check pass.

After this, the explainability feature is reliable enough to use before any further KCD1 data collection.
