//! Live-experiment tests cover both lifecycle behavior and safety invariants.
//!
//! Keep invariant tests focused on rollback availability, deterministic
//! decisions, and state transitions that affect apply-mode safety.

mod invariants;
mod lifecycle;
mod rollback;
mod runtime_executor;
mod support;
