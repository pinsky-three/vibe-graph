# Vibe-Graph

*A local-first neural OS for software projects, where specs, code and collaboration live in one evolving system, with Git as the fossil record.*

Vibe-Graph is an experimental playground for rethinking how software projects evolve. Instead of treating source code, specs, and collaboration artifacts as separate silos, Vibe-Graph maintains a living SourceCodeGraph that captures structure, relationships, and historical vibes (human + machine intents). A cellular automaton of LLM agents evolves this graph locally, while Git serves as the cold-storage layer for stable fossils.

The system is organized as a layered workspace of focused crates:

- `vibe-graph-core` – canonical domain model: graphs, vibes, constitutions, cell states, and snapshots.
- `vibe-graph-ssot` – structural perception layer that scans repos to build SourceCodeGraphs.
- `vibe-graph-semantic` – semantic/narrative mapper bridging structural graphs to conceptual regions.
- `vibe-graph-llmca` – cellular automaton fabric that evolves cell states atop the graph.
- `vibe-graph-constitution` – governance, constraints, and planning rules.
- `vibe-graph-sync` – local-first event log and future CRDT foundations.
- `vibe-graph-materializer` – turns proposed states into concrete code changes and test runs.
- `vibe-graph-git` – fossil layer that captures snapshots in Git.
- `vibe-graph-engine` – orchestrator tying together scanning, semantics, automata, constitutions, and Git.
- `vibe-graph-cli` – command-line entry point for scanning, ticking, and inspecting state.
- `vibe-graph-ui` – future visualization surface for the graph, vibes, and neural dynamics.

This repository is early-stage research code: use it to explore, remix, and iterate. Expect rapid change, incomplete features, and plenty of TODOs as we search for the right abstractions.
