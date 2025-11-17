//! Local-first event log primitives that future CRDTs can build upon.

use serde::{Deserialize, Serialize};
use vibe_graph_core::{CellState, Snapshot, Vibe};

/// Event types that can be replayed or synchronized across peers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// A new vibe was introduced.
    VibeCreated(Vibe),
    /// An existing vibe was updated.
    VibeUpdated(Vibe),
    /// A cell changed state as part of the automaton.
    CellUpdated(CellState),
    /// A snapshot suitable for fossilizing in Git was produced.
    SnapshotCreated(Snapshot),
}

/// In-memory append-only log for experimentation.
#[derive(Default, Debug)]
pub struct EventLog {
    events: Vec<Event>,
}

impl EventLog {
    /// Append a new event to the log.
    pub fn append(&mut self, event: Event) {
        self.events.push(event);
    }

    /// Iterate over events in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Event> {
        self.events.iter()
    }

    /// Inspect the current number of persisted events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Check if the log is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
