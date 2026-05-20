//! Dispatch module for agent HTTP, policy, and remote-control tests.

use super::{autotune::*, daemon::*, recording::*, routes::*, *};

mod support;

mod autotune;
mod capabilities;
mod daemon;
mod remote_policy;
mod remote_policy_limits;
mod remote_request;
mod security;
