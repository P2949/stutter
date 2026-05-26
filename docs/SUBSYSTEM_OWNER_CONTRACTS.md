# Subsystem Owner Contracts

These contracts summarize where code belongs, what each subsystem must avoid,
what errors it should return, and which tests protect the boundary.

## Actions

Owns reversible system mutations: nice, I/O priority, uclamp, CPU/IRQ affinity,
cgroups, power profiles, VM knobs, action runners, rollback tokens, and audit
events.

Must never bypass daemon policy, mutate without a rollback token when rollback is
possible, panic on token mismatch, or call CLI/command parsing code.

Errors must use `ActionError`, `PartialApplyError`, or context-rich `anyhow`
errors at lower boundaries. Wrong rollback-token kinds should become structured
rollback failures, not panics.

Protected by action unit tests, runner tests, rollback tests, typed-id
architecture tests, panic-path architecture tests, and dependency-boundary tests.

## Daemon

Owns long-running state, policy evaluation, lifecycle transitions, health,
watchdog checks, privilege boundaries, and persistent daemon-store updates.

Must never import CLI parsing, mutate persistent state outside daemon state/store
helpers, or approve mutations that bypass `DaemonPolicy`.

Errors should be policy rejections, daemon health/fault states, store errors, or
explicit lifecycle failures that can be surfaced to the agent/CLI.

Protected by daemon policy tests, daemon state architecture tests, privilege
worker tests, acceptance/soak tests, and dependency-boundary tests.

## Autotune

Owns observation, objective evaluation, candidate planning, live experiments,
baseline/comparison, history, startup recovery, and low-risk apply workflows.

Must never execute actions directly from planner/observation modules, compare
diagnostic raw score totals as objective truth, or continue applying while
quality/safety gates are failed.

Errors should be candidate denials, objective guard failures, data-quality
reasons, recovery outcomes, or action-runner errors with enough context to
explain keep/revert decisions.

Protected by planner tests, runtime/controller tests, objective architecture
tests, raw-score architecture tests, quality tests, startup recovery tests, and
dependency-boundary tests.

## eBPF

Owns no-std tracepoint programs, event ABI producers, map capacities, runtime
drop counters, and verifier-friendly event construction.

Must never change ABI struct layout without compile-time assertions, grow map
state without capacity documentation, or hide event loss without a drop counter.

Errors are represented as dropped events, drop counters, optional tracepoint
availability, or userspace loader/preflight failures.

Protected by common ABI layout assertions, eBPF layout architecture tests,
loader/preflight tests, map-sizing tests, drop-counter metadata tests, and
privileged smoke recipes.

## Report

Owns offline report loading, report models, analysis, diffing, text/HTML/JSON
rendering, regression fixtures, and uncertainty/degraded-evidence presentation.

Must never depend on live runtime/control-plane modules, require optional probes
for old artifacts, or panic on incomplete evidence.

Errors should be load errors, invalid model errors, degraded evidence, validation
warnings, or render/diff diagnostics that preserve old artifact readability.

Protected by report regression tests, golden text tests, validation corpus tests,
artifact contract tests, and report dependency-boundary tests.

## Config

Owns config model/layers, defaults, static validation, merge semantics,
effective provenance, and config-file/CLI bridging while migration is in flight.

Must never silently reinterpret old config fields, mix runtime/kernel checks into
static validation, or let merge precedence become unobservable.

Errors should name the exact field and whether the failure is static validation,
runtime validation, parse/mapping, or merge conflict/provenance.

Protected by config file tests, merge tests, effective/provenance tests,
dependency hygiene, and `docs/CONFIG_CRATE_MIGRATION.md`.

## Agent

Owns remote/local control routes, auth, rate limits, socket binding, daemon
embedding, recording/autotune route adapters, and response schemas.

Must never expose mutating routes without policy/auth checks, allow non-loopback
unsafe binds without explicit opt-in, or bypass daemon/action safety gates.

Errors should be route rejections, auth failures, policy denials, daemon faults,
rate-limit responses, or schema-stable JSON error payloads.

Protected by agent route/auth tests, unix-socket tests, remote policy tests,
daemon embedding tests, response-shape tests, and panic-path architecture tests.
