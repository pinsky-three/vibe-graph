//! CLI command implementations.

pub mod architect;
pub mod automaton;
pub mod compose;
pub mod config;
pub mod process;
pub mod quality;
pub mod remote;
pub mod run;
pub mod rustify;
pub mod semantic;
pub mod serve;

#[cfg(feature = "native-viz")]
pub mod viz;
