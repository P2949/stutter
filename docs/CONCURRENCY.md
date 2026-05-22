# Concurrency model

## Runtime model

`stutter` runs on Tokio for agent HTTP handling, daemon orchestration, and monitor sessions. Long-lived tasks are explicit owners of their mutable runtime state; shared handles are used for request coordination and cancellation rather than broad shared mutation.

## Daemon state ownership

Daemon state is owned by daemon runtime/store paths. `DaemonStateStore` is a single-owner mutable state store: callers mutate it through `&mut self`, and snapshots are replaced as a complete `DaemonState`.

`DaemonStateStore` is not intended to be wrapped in arbitrary `Arc<Mutex<_>>`. Mutations should flow through daemon runtime, policy, lifecycle, and privilege boundaries so state transitions stay auditable.

## DaemonStateStore mutation rules

`DaemonStateStore::replace` is the persistence boundary. It writes the replacement snapshot first when a writer exists, then updates in-memory state. Future changes that add locking or split ownership must update this document and keep persistence ordering explicit.

## Agent/server task model

The agent server shares `AgentState` through Axum state. Its `Mutex` fields guard short critical sections for active recording handles, active autotune handles, daemon status snapshots, and rate-limiter counters. Request handlers should not hold these locks across unrelated host mutations.

Unix socket connections are handled by bounded `tokio::spawn` tasks. A semaphore limits concurrent connections, and per-connection state is owned by the spawned task until it exits.

## Channel boundaries

`oneshot` channels are used for cancellation or one-time alignment results. `mpsc` channels are used for monitor event fanout and alert delivery. Producers should prefer bounded channels and explicit full/closed handling so telemetry cannot grow memory without limit.

## Locking rules

Use locks only at ownership boundaries where shared state is unavoidable. Prefer owned mutable structs inside daemon runtime and monitor sessions. Avoid holding `Mutex` guards across network calls, filesystem mutation, kernel/host mutation, or long-running awaits unless the code owns that serialization contract.

## Blocking filesystem and kernel-state operations

Blocking filesystem setup and kernel/host mutation must stay serialized by daemon policy, privilege worker, and runtime boundaries. Blocking sensor reads use `spawn_blocking` so they do not stall the async runtime.

## Shutdown/cancellation model

Long-running recording and autotune tasks receive `oneshot` cancellation handles and store their `JoinHandle` in agent state. Stop/reap paths are responsible for signaling cancellation and observing task completion.

## Testing expectations

Architecture tests check that this document exists and names the major concurrency primitives. Tests for agent and daemon paths should prefer explicit channel/handle assertions over timing-dependent sleeps, and should cover cancellation, reaping, and bounded-connection behavior where possible.
