I checked `stutter-experimental(9).zip` and the same four concerns are still the ones to address. Here is a step-by-step fix guide, ordered from safest/quickest to biggest architecture cleanup.

---

# 0. Current verified state

In the uploaded snapshot:

```text
68 Rust files have `Transitional` on line 1.
69 Rust files have `Transitional` in the first few lines.
stutter/src/agent/autotune.rs is 1,245 lines.
stutter/src/architecture_tests/allowlists.rs explicitly allows agent/autotune.rs at 1,245 lines.
docs/ROADMAP.md still marks “Limited auto-tuner” and “Broader optimizer” as Future.
stutter/src/affinity.rs:368 still has a production unwrap().
```

The important distinction: these are mostly **technical-debt and architecture hygiene issues**, not immediate runtime correctness bugs.

---

# Implementation progress

- [x] `stutter/src/affinity.rs` restore identity `unwrap()` removed with a `let ... else` destructure. Verified with `cargo test -p stutter affinity`.
- [x] `docs/ROADMAP.md` autotune status clarified. Verified with `rg -n "Limited auto-tuner|Broader optimizer|No broad privileged auto-optimizer" docs README.md stutter/src`.
- [x] Transitional modules inventoried with a ratcheting ceiling and local report script. Verified with `scripts/list-transitional-modules.sh` and `cargo test -p stutter architecture_tests`.
- [x] Transitional markers include reason and exit criteria. Verified by `architecture_tests::transitional::temporary_migration_markers_are_tracked`.
- [x] `stutter/src/agent/autotune.rs` moved into `stutter/src/agent/autotune/mod.rs` and split into focused modules: `policy`, `state`, `reap`, `status`, `start`, `stop`, `restore`, `history`, and `config`.
- [x] Oversized-file allowlist updated and lowered for the agent autotune split. The old `src/agent/autotune.rs` allowance was removed because `src/agent/autotune/mod.rs` is now a small façade.
- [x] Final formatting, tests, and clippy completed. Verified with `cargo fmt --all --check`, `cargo check -p stutter`, `cargo test -q -p stutter --all-targets -- --test-threads=1`, and `cargo clippy --all-targets -- -D warnings`.

---

# 1. Fix the `affinity.rs` production `unwrap()`

## Problem

In `stutter/src/affinity.rs`, the code checks that three `Option`s are all present:

```rust
if process_pid.is_none() || process_starttime_ticks.is_none() || task_starttime_ticks.is_none()
{
    return Ok(RestoreRecordStatus::IdentityMismatch);
}

let process_pid = process_pid.unwrap();
```

This is safe because of the guard, but the architecture scanner expects production unwraps to either be removed or documented with an invariant comment.

## Best fix: remove the unwrap entirely

Replace this:

```rust
if process_pid.is_none() || process_starttime_ticks.is_none() || task_starttime_ticks.is_none()
{
    return Ok(RestoreRecordStatus::IdentityMismatch);
}

let process_pid = process_pid.unwrap();
```

with this:

```rust
let (Some(process_pid), Some(process_starttime_ticks), Some(task_starttime_ticks)) =
    (process_pid, process_starttime_ticks, task_starttime_ticks)
else {
    return Ok(RestoreRecordStatus::IdentityMismatch);
};
```

That is cleaner than adding a comment because it removes the `unwrap()` completely.

## Check nearby usage

After the replacement, make sure later code still uses:

```rust
process_pid
process_starttime_ticks
task_starttime_ticks
```

as plain values, not `Option`s.

## Alternative minimal fix

If you want the smallest possible patch, add the scanner-compatible comment:

```rust
// invariant: partial identity records returned above, so process_pid is present here.
let process_pid = process_pid.unwrap();
```

But I would use the `let ... else` version. It is safer and more idiomatic.

## Test command

Run:

```bash
cargo test -p stutter affinity
cargo test -p stutter architecture_tests
```

---

# 2. Create a transitional-module inventory test/report

## Problem

The stated concern said there were 11 transitional files, but the actual count is much higher:

```text
68 files with `Transitional` on line 1
69 files with `Transitional` in the first few lines
```

That means this is no longer a tiny temporary refactor marker. It is a real surface area where warnings may be suppressed and old migration code can accumulate.

The goal is **not** to delete all transitional modules immediately. The goal is to make them visible, tracked, and periodically reduced.

## Step 2.1 — Add a central transitional inventory allowlist

Create a new file:

```text
stutter/src/architecture_tests/transitional_allowlist.rs
```

Suggested contents:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionalModuleAllowance {
    pub path: &'static str,
    pub reason: &'static str,
    pub exit_criteria: &'static str,
}

pub const TRANSITIONAL_MODULE_ALLOWLIST: &[TransitionalModuleAllowance] = &[
    TransitionalModuleAllowance {
        path: "src/process/mod.rs",
        reason: "process module split is still exposing compatibility re-exports",
        exit_criteria: "remove once direct imports no longer depend on process/mod.rs façade",
    },
    TransitionalModuleAllowance {
        path: "src/schemas/mod.rs",
        reason: "schema module split is still exposing compatibility re-exports",
        exit_criteria: "remove once callers import concrete schema modules directly",
    },
    TransitionalModuleAllowance {
        path: "src/daemon/state_compat.rs",
        reason: "daemon state compatibility bridge remains during state model migration",
        exit_criteria: "remove once all daemon state callers use the canonical model",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/matcher.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile matcher callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/model.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile model callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/cache.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile cache callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/plan.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile planning callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/apply.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile application callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/profiles/verify.rs",
        reason: "profile split compatibility stage",
        exit_criteria: "remove once profile verification callers target the final module boundary",
    },
    TransitionalModuleAllowance {
        path: "src/events/domain.rs",
        reason: "event domain compatibility stage",
        exit_criteria: "remove once event callers use final domain modules directly",
    },
    TransitionalModuleAllowance {
        path: "src/community_rules/import/validate.rs",
        reason: "community rules import validation split remains transitional",
        exit_criteria: "remove once import validation API is final",
    },
];
```

This is just the seed. Since the real count is 69, you can either:

1. Add all 69 entries immediately, or
2. Start with the known high-risk ones and add a test that prints all untracked transitional files.

I recommend option 2 for the first patch because it avoids a huge noisy allowlist commit.

## Step 2.2 — Add a scanner test

In your architecture tests, add a test that scans Rust files for `Transitional` in the first few lines.

Pseudo-code:

```rust
#[test]
fn transitional_modules_are_tracked() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");

    let mut discovered = Vec::new();

    for path in walkdir::WalkDir::new(&root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "rs"))
    {
        let text = std::fs::read_to_string(path.path()).unwrap();
        let first_lines = text.lines().take(5).collect::<Vec<_>>().join("\n");

        if first_lines.contains("Transitional") {
            let relative = path
                .path()
                .strip_prefix(&root)
                .unwrap()
                .display()
                .to_string();

            discovered.push(format!("src/{relative}"));
        }
    }

    discovered.sort();

    let allowed = TRANSITIONAL_MODULE_ALLOWLIST
        .iter()
        .map(|entry| entry.path)
        .collect::<BTreeSet<_>>();

    let untracked = discovered
        .iter()
        .filter(|path| !allowed.contains(path.as_str()))
        .collect::<Vec<_>>();

    assert!(
        untracked.is_empty(),
        "untracked transitional modules:\n{}",
        untracked
            .iter()
            .map(|path| format!("  - {path}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
```

## Step 2.3 — Add a count ceiling

This is the part that prevents accumulation.

Add a maximum count:

```rust
const MAX_TRANSITIONAL_MODULES: usize = 69;
```

Then assert:

```rust
assert!(
    discovered.len() <= MAX_TRANSITIONAL_MODULES,
    "transitional module count increased from ceiling {MAX_TRANSITIONAL_MODULES} to {}",
    discovered.len()
);
```

Once you remove transitional modules, lower the ceiling.

Example:

```text
Current: 69
After cleanup: 64
Update ceiling: 64
```

This turns transitional cleanup into ratcheting progress.

## Step 2.4 — Add exit criteria to each transitional comment

Right now many files probably say something like:

```rust
// Transitional: ...
#![allow(dead_code)]
```

Improve them to include an exit condition:

```rust
// Transitional: compatibility façade during profile split.
// Exit: remove once callers import profiles::{model, matcher, cache, plan, apply, verify}
// directly and this module no longer owns any unique logic.
#![allow(dead_code)]
```

The key rule:

```text
Every transitional marker should answer:
1. Why does this exist?
2. What warning is being suppressed?
3. What has to happen before it can be removed?
```

## Step 2.5 — Sweep easiest transitional files first

Start with files that are pure re-export façades.

Command:

```bash
grep -R "^//.*Transitional" -n stutter/src | sort
```

Then for each candidate:

1. Check whether the file only re-exports items.
2. Search all imports.
3. Update callers to import from final module paths.
4. Remove the façade or remove its `allow`.
5. Run targeted tests.

Good first targets are usually:

```text
process/mod.rs
schemas/mod.rs
events/domain.rs
profiles/*.rs façade files
```

Do **not** start with complex modules that still contain logic. Start with pure compatibility surfaces.

## Test commands

```bash
cargo test -p stutter architecture_tests
cargo test -p stutter --all-targets
```

---

# 3. Split `stutter/src/agent/autotune.rs`

## Problem

`agent/autotune.rs` is exactly 1,245 lines and is explicitly allowed in:

```text
stutter/src/architecture_tests/allowlists.rs
```

with this reason:

```text
autotune agent route handlers, remote policy helpers, explicit task reaping status,
active record-level restore endpoint wiring, task reaping, and enum-mode apply-low-risk
start behavior remain pending future policy/helper split
```

That allowlist entry is honest, but it confirms the file is doing too much.

## Goal

Turn:

```text
stutter/src/agent/autotune.rs
```

into a thin module boundary that delegates to focused files.

Suggested final layout:

```text
stutter/src/agent/autotune/
  mod.rs
  policy.rs
  state.rs
  reap.rs
  start.rs
  stop.rs
  restore.rs
  status.rs
  history.rs
  config.rs
```

You do not need to do all of this in one patch. Do it slowly.

---

## Step 3.1 — Create the module directory

Create:

```text
stutter/src/agent/autotune/
```

Then move the old file to:

```text
stutter/src/agent/autotune/mod.rs
```

Rust conflict warning: you cannot have both:

```text
agent/autotune.rs
agent/autotune/mod.rs
```

at the same time.

So this step requires either:

```bash
mkdir -p stutter/src/agent/autotune
git mv stutter/src/agent/autotune.rs stutter/src/agent/autotune/mod.rs
```

Then run:

```bash
cargo check -p stutter
```

Before any actual split, this should compile if module declarations remain compatible.

---

## Step 3.2 — Split low-risk pure helpers first

Do **not** start by moving route handlers. First move pure helper code.

Create:

```text
stutter/src/agent/autotune/policy.rs
```

Move types/functions related to:

```text
AutotuneStartSecurityRejection
remote mode rejection
policy checks
daemon/autotune mode validation
apply-low-risk start mode conversion
```

Example module boundary:

```rust
// stutter/src/agent/autotune/policy.rs

use super::*;

pub(crate) struct AutotuneStartSecurityRejection {
    pub(crate) status: StatusCode,
    pub(crate) audit_message: String,
    pub(crate) response_message: String,
}

pub(crate) fn reject_remote_apply_low_risk_if_needed(...) -> Option<AutotuneStartSecurityRejection> {
    ...
}
```

In `mod.rs`:

```rust
mod policy;

use policy::{
    AutotuneStartSecurityRejection,
    reject_remote_apply_low_risk_if_needed,
};
```

Run:

```bash
cargo fmt
cargo test -p stutter agent
```

---

## Step 3.3 — Split task reaping

Create:

```text
stutter/src/agent/autotune/reap.rs
```

Move code related to:

```text
task reaping
explicit task reaping status
stale process cleanup
autotune task state cleanup
```

Keep route handlers in `mod.rs` initially. The route handler can call:

```rust
reap::reap_autotune_task(...)
```

Suggested boundary:

```rust
pub(crate) struct ReapAutotuneTaskOutcome {
    pub(crate) reaped: bool,
    pub(crate) previous_status: Option<String>,
    pub(crate) message: String,
}

pub(crate) fn reap_autotune_task(...) -> ReapAutotuneTaskOutcome {
    ...
}
```

Why this split matters: reaping logic is easy to accidentally mix with status/start/restore behavior. Isolating it reduces side effects.

Run:

```bash
cargo test -p stutter agent
cargo test -p stutter autotune
```

---

## Step 3.4 — Split restore endpoint code next

This should be the highest-priority functional split because restore bugs are safety-sensitive.

Create:

```text
stutter/src/agent/autotune/restore.rs
```

Move code related to:

```text
active record-level restore endpoint wiring
restore request parsing
restore action selection
restore response rendering
manual restore command reporting
```

Keep the HTTP handler signature stable. The route handler can be tiny:

```rust
pub(crate) async fn restore_autotune_handler(...) -> impl IntoResponse {
    restore::handle_restore_request(...).await
}
```

Inside `restore.rs`, expose one main function:

```rust
pub(crate) async fn handle_restore_request(...) -> Result<Response, AutotuneAgentError> {
    ...
}
```

Acceptance criteria for this split:

```text
agent/autotune/mod.rs should not know restore internals.
restore.rs should own restore-specific validation and response mapping.
emergency restore behavior should stay in autotune/emergency_restore.rs, not in agent.
```

Run:

```bash
cargo test -p stutter autotune::emergency_restore
cargo test -p stutter agent
```

---

## Step 3.5 — Split start/stop/status routes

After restore is isolated, split the route groups.

Create:

```text
stutter/src/agent/autotune/start.rs
stutter/src/agent/autotune/stop.rs
stutter/src/agent/autotune/status.rs
```

Move:

```text
start endpoint
stop endpoint
status endpoint
runtime-mode conversion
start response rendering
status response rendering
```

Do this one module per commit.

Recommended order:

```text
1. status.rs
2. stop.rs
3. start.rs
```

Why start last? Start usually has the most policy branches.

---

## Step 3.6 — Split history/config routes

Create:

```text
stutter/src/agent/autotune/history.rs
stutter/src/agent/autotune/config.rs
```

Move only route-specific code. Do not move core autotune config model types out of their existing canonical modules.

---

## Step 3.7 — Reduce the oversized-file allowlist ceiling

After each split, lower the allowlist.

Current:

```rust
OversizedRustFileAllowance {
    path: "src/agent/autotune.rs",
    max_lines: 1_245,
    reason: "...",
}
```

After moving to `mod.rs`, update path:

```rust
OversizedRustFileAllowance {
    path: "src/agent/autotune/mod.rs",
    max_lines: 900,
    reason: "temporary autotune route façade while remaining route groups move into focused submodules",
}
```

Then ratchet down:

```text
After policy.rs/reap.rs: 900
After restore.rs: 750
After status/stop/start: 500
Final target: no allowlist entry
```

## Test commands for every split

```bash
cargo fmt --check
cargo check -p stutter
cargo test -p stutter agent
cargo test -p stutter autotune
cargo test -p stutter architecture_tests
cargo clippy --all-targets -- -D warnings
```

---

# 4. Update `docs/ROADMAP.md` autotune status

## Problem

`docs/ROADMAP.md` currently says:

```text
Limited auto-tuner | Future
Broader optimizer | Future
```

But the codebase already contains substantial autotune infrastructure:

```text
runtime
daemon session
live experiments
rollback
emergency restore
active config matching
planning
external mutation recovery
startup recovery
providers for CPU affinity, nice, ioprio, uclamp, cgroup, IRQ affinity, CPU power, VM knobs, GPU power
```

So the roadmap makes the project look less implemented than it is.

## Goal

Clarify the difference between:

```text
implementation exists
```

and:

```text
user-facing safe enablement is complete
```

## Step 4.1 — Replace the roadmap rows

Current:

```markdown
| Limited auto-tuner | Future | Any future automatic action should go through safety classes, preflight, audit logging, verification, and rollback. |
| Broader optimizer | Future | IRQ affinity, uclamp, nice, GPU, SCX, and other tunables require stronger evidence and explicit safety design before implementation. |
```

Replace with:

```markdown
| Limited auto-tuner | In development / gated | Runtime, planning, live experiment, rollback, audit, and emergency-restore infrastructure exist. User-facing enablement remains gated on safety validation, policy coverage, and end-to-end recovery confidence. |
| Broader optimizer | Experimental / internal | Providers and candidate paths exist for CPU affinity, nice, ioprio, uclamp, cgroup placement, IRQ affinity, CPU power, VM knobs, and GPU power. Broader user-facing enablement remains future work pending stronger evidence, policy hardening, and clearer operator controls. |
```

## Step 4.2 — Add a note below the table

Add:

```markdown
### Autotune implementation status

The autotune subsystem is partially implemented in-tree. Roadmap status refers to
safe user-facing enablement, not absence of code. Experimental/internal paths may
exist before they are considered supported defaults.
```

## Step 4.3 — Keep “No broad privileged auto-optimizer”

The roadmap currently says:

```text
No broad privileged auto-optimizer.
```

Keep that principle, but clarify it:

```markdown
- No broad privileged auto-optimizer is enabled by default. Any automatic action
  must remain gated by safety class, preflight, audit logging, verification,
  rollback, and explicit policy.
```

This prevents people from misreading the roadmap as “autotune does not exist.”

## Documentation check

Run:

```bash
grep -R "Limited auto-tuner\|Broader optimizer\|No broad privileged auto-optimizer" -n docs README.md stutter/src
```

Make sure no other docs still claim these systems are purely future/nonexistent.

---

# 5. Add a periodic transitional sweep workflow

This is optional, but it is the best way to prevent the 69 transitional markers from becoming permanent.

## Step 5.1 — Add a local script

Create:

```text
scripts/list-transitional-modules.sh
```

Contents:

```bash
#!/usr/bin/env bash
set -euo pipefail

grep -R "Transitional" -n stutter/src --include='*.rs' \
  | sort
```

Make it executable:

```bash
chmod +x scripts/list-transitional-modules.sh
```

## Step 5.2 — Add a count mode

Better version:

```bash
#!/usr/bin/env bash
set -euo pipefail

matches="$(grep -R "Transitional" -n stutter/src --include='*.rs' | sort || true)"

if [[ -z "$matches" ]]; then
    echo "transitional modules: 0"
    exit 0
fi

echo "$matches"
echo
echo "transitional marker count: $(printf '%s\n' "$matches" | wc -l)"
```

## Step 5.3 — Add this to the refactor checklist

In docs, add:

````markdown
Before merging large refactors:

```bash
scripts/list-transitional-modules.sh
cargo test -p stutter architecture_tests
````

````

## Step 5.4 — Sweep in batches

Do not try to remove all 69 markers at once.

Use batches:

```text
Batch 1: pure re-export façades
Batch 2: compatibility wrappers with no unique tests
Batch 3: modules with dead-code allows
Batch 4: modules with unused-import allows
Batch 5: deeper transitional modules that still own logic
````

For each file:

```text
1. Identify why the transitional marker exists.
2. Search all imports/callers.
3. Move callers to final module path.
4. Remove re-export or dead code.
5. Remove allow.
6. Run targeted tests.
7. Lower transitional count ceiling.
```

---

# 6. Suggested commit sequence

Do not do this as one giant patch. Use small commits.

## Commit 1 — Remove safe unwrap

```text
affinity: replace guarded restore identity unwrap with let-else
```

Files:

```text
stutter/src/affinity.rs
```

Run:

```bash
cargo test -p stutter affinity
cargo test -p stutter architecture_tests
```

---

## Commit 2 — Track transitional modules

```text
architecture: track transitional module markers
```

Files:

```text
stutter/src/architecture_tests/transitional_allowlist.rs
stutter/src/architecture_tests.rs or equivalent mod registration
```

Run:

```bash
cargo test -p stutter architecture_tests
```

---

## Commit 3 — Update roadmap wording

```text
docs: clarify autotune roadmap status
```

Files:

```text
docs/ROADMAP.md
```

Run:

```bash
grep -R "Limited auto-tuner\|Broader optimizer" -n docs
```

---

## Commit 4 — Prepare agent autotune module split

```text
agent: move autotune handler into module directory
```

Files:

```text
stutter/src/agent/autotune.rs -> stutter/src/agent/autotune/mod.rs
```

Run:

```bash
cargo check -p stutter
cargo test -p stutter agent
```

---

## Commit 5 — Split policy helpers

```text
agent: split autotune policy helpers
```

Files:

```text
stutter/src/agent/autotune/mod.rs
stutter/src/agent/autotune/policy.rs
```

Run:

```bash
cargo test -p stutter agent
```

---

## Commit 6 — Split reaping helpers

```text
agent: split autotune task reaping helpers
```

Files:

```text
stutter/src/agent/autotune/mod.rs
stutter/src/agent/autotune/reap.rs
```

Run:

```bash
cargo test -p stutter agent
cargo test -p stutter autotune
```

---

## Commit 7 — Split restore endpoint

```text
agent: split autotune restore endpoint
```

Files:

```text
stutter/src/agent/autotune/mod.rs
stutter/src/agent/autotune/restore.rs
```

Run:

```bash
cargo test -p stutter autotune::emergency_restore
cargo test -p stutter agent
```

---

## Commit 8+ — Continue route splits

```text
agent: split autotune status endpoint
agent: split autotune stop endpoint
agent: split autotune start endpoint
agent: split autotune history endpoint
agent: split autotune config endpoint
```

Run each time:

```bash
cargo fmt --check
cargo check -p stutter
cargo test -p stutter agent
cargo clippy --all-targets -- -D warnings
```

---

# 7. Acceptance criteria

Call this cleanup complete only when these are true:

```text
[x] stutter/src/affinity.rs has no production unwrap at the restore identity site.
[x] Transitional modules are inventoried by an architecture test or script.
[x] Transitional module count has a ratcheting ceiling.
[x] Each transitional marker has a reason and exit criteria.
[x] docs/ROADMAP.md distinguishes implemented autotune infrastructure from supported user-facing enablement.
[x] agent/autotune.rs has been moved to agent/autotune/mod.rs or split into focused modules.
[x] Restore, reaping, policy, status/start/stop code are not all mixed in one 1,245-line file.
[x] architecture_tests oversized allowlist is lowered after each split.
[x] cargo fmt, cargo test, and clippy pass.
```

---

# 8. Priority order

Do them in this order:

```text
1. affinity.rs unwrap cleanup
2. ROADMAP wording fix
3. transitional inventory/count test
4. agent/autotune.rs split: policy helpers
5. agent/autotune.rs split: reaping helpers
6. agent/autotune.rs split: restore endpoint
7. agent/autotune.rs split: status/stop/start/history/config
8. transitional marker sweeps in batches
```

Why this order:

```text
affinity.rs is tiny and safe.
ROADMAP is docs-only and prevents confusion.
transitional inventory prevents further accumulation.
agent/autotune.rs split is larger and should be done gradually.
transitional sweeps are ongoing cleanup, not one emergency patch.
```
