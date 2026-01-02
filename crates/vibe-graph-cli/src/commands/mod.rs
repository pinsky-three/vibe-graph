//! CLI command implementations.

pub mod automaton;
pub mod compose;
pub mod config;
pub mod remote;
pub mod serve;

#[cfg(feature = "native-viz")]
pub mod viz;
