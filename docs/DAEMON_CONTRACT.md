# Daemon Contract

This document records the daemon/product contract enforced by the central policy code in `stutter/src/daemon_policy.rs`.

The source of truth for policy enforcement is `DaemonMode`, `DaemonPolicy`, `ActionDescriptor`, `ActionEffectScope`, and `RollbackRequirement`. This document does not enable any unsupported runtime mode. Live `stutter autotune --mode` currently supports `observe`, `suggest`, and `apply-low-risk`; `apply-medium-risk` and `apply-high-risk` are policy labels for explicitly unlocked future apply paths.

## Modes

| Mode | Contract |
| --- | --- |
| `observe` | Never changes system state. Permits monitoring, reports, dry-run/preflight checks, verify steps, and rollback operations. It does not emit suggestions as controller decisions and it does not apply actions. |
| `suggest` | Proposes changes and may emit candidate actions. It never applies changes. Suggestion output must state that suggest mode did not apply the change and must separate dry-run commands from policy-gated manual apply commands. |
| `apply-low-risk` | Applies only `SafetyClass::ReversibleLowRisk` actions whose `ActionEffectScope` is `LocalProcess` or `LocalProcessTree`, with rollback available before apply. This is the default apply ceiling. The currently implemented low-risk apply family is CPU-affinity candidates for explicit target process trees. |
| `apply-medium-risk` | Opt-in policy mode for reversible but more invasive local/process-scoped changes. It allows at most `SafetyClass::ReversibleMediumRisk` and still requires explicit target scope, rollback before apply, sufficient confidence, and non-persistent/non-system-wide defaults. |
| `apply-high-risk` | Never default. High-risk actions require explicit high-risk unlock through `DaemonPolicy::allow_high_risk`. Remote high-risk support is not available by default and must not be exposed as a default remote mode. |

## Suggestion output contract

Candidate suggestions are non-mutating.

Suggestion output must include:

- `suggest mode did not apply this change`;
- `required_mode`;
- `required_safety_class`;
- `rollback=stutter restore`;
- a dry-run command.

A manual apply command may be printed only when a `DaemonPolicy` for `ActionSource::Cli` would allow the candidate descriptor. High-risk candidates must not print direct manual apply commands until high-risk CLI unlock and documentation exist.

## Free performance invariants

A daemon action may be treated as "free performance" only when all of these invariants hold:

- no persistent degradation;
- no unrecoverable state;
- no changes without a rollback path;
- no action when confidence is low;
- no system-wide mutation by default.

`DaemonPolicy::check_action` enforces the apply-side parts of this contract by rejecting low-confidence actions, missing rollback, unavailable rollback, persistent effects without explicit permission, system-wide effects without explicit permission, and effect scopes outside the selected mode.

## Default allowed touch points

By default, stutter may touch only these areas:

- read-only `/proc` inspection, supported eBPF monitoring, and run artifacts under `~/.local/state/stutter/runs/`;
- CPU affinity of explicit target process trees for low-risk apply;
- audit, history, journal, and restore state under the stutter state directory:
  - `~/.local/state/stutter/last_profile_restore.json`;
  - `~/.local/state/stutter/last_affinity_restore.json`;
  - `~/.local/state/stutter/audit/actions.jsonl`;
  - `~/.local/state/stutter/autotune/controller_journal.json`;
  - `~/.local/state/stutter/autotune/history.jsonl`.

## Forbidden by default

These are forbidden by default:

- arbitrary system-wide process mutation;
- sysfs power knobs;
- VM knobs;
- IRQ affinity;
- GPU power settings;
- CPU power settings;
- cgroup moves;
- high-risk actions;
- foreground title capture in unsafe remote mode.

These actions require explicit policy support before use. A command-line flag, config file entry, remote request, or local helper must not bypass `DaemonPolicy::check_action`.

## Rollback contract

Rollback is part of the product contract, not a best-effort comment.

- `stutter restore` restores managed profile state from the normal restore files.
- `stutter restore --dry-run` previews pending restore work without changing live state.
- `stutter autotune restore` restores autotune controller state from `controller_journal.json`.
- The audit log at `~/.local/state/stutter/audit/actions.jsonl` records system-changing action events.
- The controller journal at `~/.local/state/stutter/autotune/controller_journal.json` records in-flight and applied autotune actions with rollback tokens.
- Startup recovery must inspect the controller journal before planning new actions.
- If target tasks exit before rollback, restore reports skipped/dead tasks instead of treating task exit as corruption.
- If task identity no longer matches, restore skips the task to avoid TID reuse damage.

## Disable path

To disable daemon/autotune behavior:

1. Run `observe` mode, for example `stutter autotune --mode observe ...`.
2. Stop any active remote/autotune controller.
3. Run `stutter restore`.
4. If the autotune controller journal contains an applied action, run `stutter autotune restore`.
5. Remove, disable, or ignore any installed agent service if one was installed.

## Developer rules

New system-changing code must follow these rules:

- every new action must implement or expose an `ActionDescriptor`;
- every new apply path must call `DaemonPolicy::check_action` before mutation;
- no new direct mutation helper may be public without a policy parameter;
- docs and CLI help must not claim a mode can apply actions before the policy gate and runtime path both support it.
