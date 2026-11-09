//! Validation-only privileged runner (development/validation infrastructure, never a product
//! component or part of the release package). A closed set of typed actions over a local Unix
//! socket, gated by kernel peer credentials and a root-owned authorization.
pub mod authorization;
pub mod boot;
pub mod cli;
pub mod client;
pub mod containment;
pub mod credentials;
pub mod delegation;
pub mod install;
pub mod model;
pub mod process;
pub mod protocol;
pub mod resume;
pub mod server;
pub mod service;
pub mod state;
mod suspend;
pub mod sys;

#[cfg(test)]
mod tests;
