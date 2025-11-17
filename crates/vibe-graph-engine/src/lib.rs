//! Orchestrator that wires scanners, automata, constitutions, and git fossils together.

use std::time::SystemTime;

use anyhow::Result;
use tracing::info;
use vibe_graph_constitution::ConstitutionEngine;
use vibe_graph_core::{CellState, Constitution, Snapshot, SourceCodeGraph, Vibe};
use vibe_graph_git::{GitBackend, GitFossilStore};
use vibe_graph_llmca::{CellUpdateRule, LlmcaSystem, NoOpUpdateRule};
use vibe_graph_sync::{Event, EventLog};

/// Configuration describing how the engine should boot.
pub struct EngineConfig {
    /// Graph the engine should orchestrate.
    pub graph: SourceCodeGraph,
    /// Constitution that governs updates.
    pub constitution: Constitution,
    /// Backend responsible for fossilizing snapshots.
    pub git_backend: GitBackend,
    /// Rule that drives the cellular automaton updates.
    pub update_rule: Box<dyn CellUpdateRule + Send + Sync>,
}

impl EngineConfig {
    /// Convenience constructor using the no-op update rule, useful for testing.
    pub fn with_noop_rule(
        graph: SourceCodeGraph,
        constitution: Constitution,
        git_backend: GitBackend,
    ) -> Self {
        Self {
            graph,
            constitution,
            git_backend,
            update_rule: Box::new(NoOpUpdateRule),
        }
    }
}

/// Coordinates all runtime subsystems for the Vibe-Graph neural OS.
pub struct VibeGraphEngine {
    graph: SourceCodeGraph,
    llmca: LlmcaSystem,
    constitution_engine: ConstitutionEngine,
    event_log: EventLog,
    git_backend: GitBackend,
    pending_vibes: Vec<Vibe>,
}

impl VibeGraphEngine {
    /// Build an engine instance from the provided configuration.
    pub fn new(config: EngineConfig) -> Result<Self> {
        let EngineConfig {
            graph,
            constitution,
            git_backend,
            update_rule,
        } = config;

        let llmca = LlmcaSystem::new(graph.clone(), update_rule);
        let constitution_engine = ConstitutionEngine::new(constitution.clone());

        Ok(Self {
            graph,
            llmca,
            constitution_engine,
            event_log: EventLog::default(),
            git_backend,
            pending_vibes: Vec::new(),
        })
    }

    /// Apply an incoming vibe so it can influence future ticks.
    pub fn apply_vibe(&mut self, vibe: Vibe) -> Result<()> {
        info!(id = %vibe.id, title = %vibe.title, "apply_vibe");
        self.event_log.append(Event::VibeCreated(vibe.clone()));
        self.pending_vibes.push(vibe);
        Ok(())
    }

    /// Advance the system by one automaton tick.
    pub fn tick(&mut self) -> Result<()> {
        let constitution = self.constitution_engine.constitution().clone();
        self.llmca.tick(&self.pending_vibes, &constitution)?;
        Ok(())
    }

    /// Produce and persist a snapshot of the current state.
    pub fn snapshot(&mut self) -> Result<Snapshot> {
        let constitution = self.constitution_engine.constitution().clone();
        let cell_states: Vec<CellState> = self.llmca.cell_states();
        let snapshot = Snapshot {
            id: format!("snapshot-{}", self.event_log.len() + 1),
            graph: self.graph.clone(),
            vibes: self.pending_vibes.clone(),
            cell_states,
            constitution,
            created_at: SystemTime::now(),
        };

        self.git_backend.commit_snapshot(&snapshot)?;
        self.event_log
            .append(Event::SnapshotCreated(snapshot.clone()));

        Ok(snapshot)
    }

    /// Access the event log for inspection/testing.
    pub fn events(&self) -> &EventLog {
        &self.event_log
    }
}
