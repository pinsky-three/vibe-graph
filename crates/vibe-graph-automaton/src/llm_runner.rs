//! LLM-powered rule execution using Rig.
//!
//! This module provides an LLM-based `Rule` implementation that uses the
//! [Rig](https://github.com/0xPlaygrounds/rig) library to compute state transitions.
//!
//! The core idea mirrors the `dynamical-system` pattern:
//! - Each node's state + neighbors are sent to an LLM
//! - The LLM returns a structured `(rule, state)` pair
//! - The automaton applies this as a transition
//!
//! ## Features
//!
//! - **Multiple providers**: OpenAI, Ollama, Anthropic, etc. via Rig's unified interface
//! - **Structured outputs**: Type-safe JSON responses
//! - **Resolver pool**: Round-robin distribution across multiple LLM endpoints
//! - **Async support**: Full async/await for parallel evolution
//!
//! ## Example
//!
//! ```rust,ignore
//! use rig::providers::openai;
//! use vibe_graph_automaton::{LlmRule, GraphAutomaton};
//!
//! let client = openai::Client::from_env();
//! let agent = client.agent("gpt-4o").build();
//! let rule = LlmRule::new(agent);
//!
//! let mut automaton = GraphAutomaton::new(temporal_graph)
//!     .with_rule(Arc::new(rule));
//!
//! // Async tick
//! automaton.tick_async().await?;
//! ```

use std::env;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use rig::agent::Agent;
use rig::completion::{CompletionModel, Prompt};
use rig::providers::openai;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::debug;

use crate::error::{AutomatonError, AutomatonResult};
use crate::rule::{RuleContext, RuleId};
use crate::state::StateData;

// =============================================================================
// Structured Output Schema
// =============================================================================

/// The structured output expected from the LLM.
///
/// This mirrors the `CognitiveUnitPair` from `dynamical-system`:
/// ```json
/// { "rule": "rule_description", "state": "state_json" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextStateOutput {
    /// The rule that should be applied (can be same or new).
    pub rule: String,

    /// The next state as a JSON value.
    pub state: Value,

    /// Optional activation level (0.0 to 1.0).
    #[serde(default)]
    pub activation: Option<f32>,

    /// Optional feedback/reasoning from the LLM.
    #[serde(default)]
    pub feedback: Option<String>,
}

impl Default for NextStateOutput {
    fn default() -> Self {
        Self {
            rule: String::new(),
            state: Value::Null,
            activation: None,
            feedback: None,
        }
    }
}

impl NextStateOutput {
    /// Get the JSON schema description for prompts.
    pub fn schema_description() -> &'static str {
        r#"{
  "rule": "string - the rule/behavior description for this unit",
  "state": "any - the next state as JSON (string, object, array, etc.)",
  "activation": "number (optional) - activation level from 0.0 to 1.0",
  "feedback": "string (optional) - reasoning or notes about the transition"
}"#
    }
}

// =============================================================================
// LLM Resolver Configuration
// =============================================================================

/// Configuration for an LLM endpoint.
///
/// Supports OpenAI-compatible APIs (OpenAI, Ollama, vLLM, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResolver {
    /// Base URL for the API (e.g., "https://api.openai.com/v1")
    pub api_url: String,

    /// API key for authentication
    pub api_key: String,

    /// Model name to use (e.g., "gpt-4o", "llama3", "phi3")
    pub model_name: String,
}

impl LlmResolver {
    /// Create a new resolver.
    pub fn new(
        api_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            api_url: api_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
        }
    }

    /// Create a resolver for local Ollama.
    pub fn ollama(model_name: impl Into<String>) -> Self {
        Self {
            api_url: "http://localhost:11434/v1".to_string(),
            api_key: "ollama".to_string(),
            model_name: model_name.into(),
        }
    }

    /// Create a resolver from environment variables.
    pub fn from_env() -> Option<Self> {
        let api_url = env::var("OPENAI_API_URL")
            .or_else(|_| env::var("VIBE_GRAPH_LLM_API_URL"))
            .ok()?;
        let api_key = env::var("OPENAI_API_KEY")
            .or_else(|_| env::var("VIBE_GRAPH_LLM_API_KEY"))
            .ok()?;
        let model_name = env::var("OPENAI_MODEL_NAME")
            .or_else(|_| env::var("VIBE_GRAPH_LLM_MODEL"))
            .unwrap_or_else(|_| "gpt-4o-mini".to_string());

        Some(Self {
            api_url,
            api_key,
            model_name,
        })
    }

    /// Load multiple resolvers from environment (comma-separated).
    pub fn load_from_env() -> Vec<Self> {
        let urls = env::var("VIBE_GRAPH_LLM_API_URLS")
            .or_else(|_| env::var("OPENAI_API_URL"))
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());

        let keys = env::var("VIBE_GRAPH_LLM_API_KEYS")
            .or_else(|_| env::var("OPENAI_API_KEY"))
            .unwrap_or_else(|_| "ollama".to_string());

        let models = env::var("VIBE_GRAPH_LLM_MODELS")
            .or_else(|_| env::var("OPENAI_MODEL_NAME"))
            .unwrap_or_else(|_| "phi3".to_string());

        let urls: Vec<&str> = urls.split(',').map(|s| s.trim()).collect();
        let keys: Vec<&str> = keys.split(',').map(|s| s.trim()).collect();
        let models: Vec<&str> = models.split(',').map(|s| s.trim()).collect();

        // Zip them together, cycling shorter lists
        let max_len = urls.len().max(keys.len()).max(models.len());

        (0..max_len)
            .map(|i| Self {
                api_url: urls[i % urls.len()].to_string(),
                api_key: keys[i % keys.len()].to_string(),
                model_name: models[i % models.len()].to_string(),
            })
            .collect()
    }

    /// Load resolvers from a TOML file.
    pub fn load_from_toml<P: AsRef<Path>>(path: P) -> AutomatonResult<Vec<Self>> {
        #[derive(Deserialize)]
        struct TomlConfig {
            resolvers: Vec<LlmResolver>,
        }

        let content =
            fs::read_to_string(path.as_ref()).map_err(|e| AutomatonError::GraphConstruction {
                message: format!("Failed to read resolver config: {}", e),
            })?;

        let config: TomlConfig =
            toml::from_str(&content).map_err(|e| AutomatonError::GraphConstruction {
                message: format!("Invalid resolver TOML: {}", e),
            })?;

        Ok(config.resolvers)
    }
}

// =============================================================================
// Resolver Pool
// =============================================================================

/// A pool of LLM resolvers with round-robin selection.
///
/// This enables distributed computation across multiple LLM endpoints,
/// similar to the `dynamical-system` pattern.
#[derive(Debug)]
pub struct ResolverPool {
    resolvers: Vec<LlmResolver>,
    cursor: AtomicUsize,
}

impl ResolverPool {
    /// Create a new pool from resolvers.
    pub fn new(resolvers: Vec<LlmResolver>) -> Self {
        Self {
            resolvers,
            cursor: AtomicUsize::new(0),
        }
    }

    /// Create from environment variables.
    pub fn from_env() -> Self {
        Self::new(LlmResolver::load_from_env())
    }

    /// Get the next resolver (round-robin).
    pub fn next(&self) -> Option<&LlmResolver> {
        if self.resolvers.is_empty() {
            return None;
        }
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed);
        Some(&self.resolvers[idx % self.resolvers.len()])
    }

    /// Get a specific resolver by index.
    pub fn get(&self, index: usize) -> Option<&LlmResolver> {
        self.resolvers.get(index)
    }

    /// Number of resolvers in the pool.
    pub fn len(&self) -> usize {
        self.resolvers.len()
    }

    /// Check if pool is empty.
    pub fn is_empty(&self) -> bool {
        self.resolvers.is_empty()
    }
}

// =============================================================================
// LLM Rule Implementation
// =============================================================================

/// System prompt template for the cognitive unit.
const DEFAULT_SYSTEM_PROMPT: &str = r#"You are an LLM Cognitive Unit in a graph automaton. Your task is to compute your next state based on:
1. Your current state and rule
2. Your memory (recent history of states)
3. Your neighbors' current states

Respond ONLY with valid JSON matching this schema:
{schema}

Guidelines:
- If your rule is empty, propose a meaningful rule based on context
- The state can be any JSON value (string, number, object, array)
- Keep states concise but informative
- Consider neighbor states when computing your next state
- Activation should reflect confidence/energy (0.0 to 1.0)

Do NOT include explanations, markdown, or code blocks. Return only the raw JSON."#;

/// An LLM-powered Rule that uses Rig for inference.
pub struct LlmRule<M: CompletionModel> {
    /// The Rig agent/model to use.
    agent: Agent<M>,

    /// Custom system prompt (optional).
    system_prompt: Option<String>,

    /// Rule identifier.
    rule_id: RuleId,

    /// Memory window size to include in prompts.
    memory_window: usize,
}

impl<M: CompletionModel> LlmRule<M> {
    /// Create a new LLM rule with a Rig agent.
    pub fn new(agent: Agent<M>) -> Self {
        Self {
            agent,
            system_prompt: None,
            rule_id: RuleId::new("llm_cognitive_unit"),
            memory_window: 4,
        }
    }

    /// Set a custom system prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Set the rule identifier.
    pub fn with_rule_id(mut self, id: impl Into<RuleId>) -> Self {
        self.rule_id = id.into();
        self
    }

    /// Set memory window size.
    pub fn with_memory_window(mut self, window: usize) -> Self {
        self.memory_window = window.max(1);
        self
    }

    /// Build the system prompt.
    fn build_system_prompt(&self) -> String {
        let base = self
            .system_prompt
            .clone()
            .unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string());

        base.replace("{schema}", NextStateOutput::schema_description())
    }

    /// Build the user message from context.
    fn build_user_message(&self, ctx: &RuleContext) -> String {
        // Current state
        let current = ctx.state.current();
        let current_json = json!({
            "rule": current.rule_id.name(),
            "state": current.state.payload.clone(),
            "activation": current.state.activation,
        });

        // Recent history
        let history: Vec<Value> = ctx
            .state
            .recent_history(self.memory_window)
            .iter()
            .map(|t| {
                json!({
                    "rule": t.rule_id.name(),
                    "state": t.state.payload.clone(),
                    "activation": t.state.activation,
                })
            })
            .collect();

        // Neighbors
        let neighbors: Vec<Value> = ctx
            .neighbors
            .iter()
            .map(|n| {
                let state = n.state.current_state();
                json!({
                    "relationship": n.relationship.clone(),
                    "rule": n.state.current_rule().name(),
                    "state": state.payload.clone(),
                    "activation": state.activation,
                })
            })
            .collect();

        // Build full context
        let context = json!({
            "current": current_json,
            "memory": history,
            "neighbors": neighbors,
            "tick": ctx.tick,
        });

        serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse LLM response into NextStateOutput.
    fn parse_response(&self, response: &str) -> AutomatonResult<NextStateOutput> {
        // Clean up common LLM response artifacts
        let cleaned = response
            .trim()
            .trim_matches('`')
            .trim_start_matches("json")
            .trim_start_matches("JSON")
            .trim_matches(['`', ' ', '\n', '\r'])
            .trim();

        serde_json::from_str(cleaned).map_err(|e| AutomatonError::StateSerialization(e))
    }

    /// Execute LLM inference asynchronously.
    pub async fn infer_async(&self, ctx: &RuleContext<'_>) -> AutomatonResult<NextStateOutput> {
        let _system_prompt = self.build_system_prompt();
        let user_message = self.build_user_message(ctx);

        // Build the full prompt including system context
        let full_prompt = format!(
            "{}\n\n---\n\nCurrent context:\n{}",
            self.build_system_prompt(),
            user_message
        );

        debug!(
            node_id = ctx.node_id.0,
            tick = ctx.tick,
            "llm_rule_inference_start"
        );

        // Use Rig's completion interface
        let response = self.agent.prompt(full_prompt).await.map_err(|e| {
            AutomatonError::RuleExecutionFailed {
                rule_id: self.rule_id.clone(),
                node_id: ctx.node_id,
                message: format!("LLM call failed: {}", e),
            }
        })?;

        debug!(
            node_id = ctx.node_id.0,
            response_len = response.len(),
            "llm_rule_inference_complete"
        );

        self.parse_response(&response)
    }

    /// Convert NextStateOutput to StateData.
    pub fn to_state_data(&self, output: NextStateOutput, current: &StateData) -> StateData {
        StateData {
            payload: output.state,
            activation: output.activation.unwrap_or(current.activation),
            annotations: if let Some(feedback) = output.feedback {
                let mut ann = current.annotations.clone();
                ann.insert("llm:feedback".to_string(), feedback);
                ann
            } else {
                current.annotations.clone()
            },
        }
    }
}

// Note: We can't implement the sync Rule trait directly since LLM calls are async.
// Instead, we provide async methods and a wrapper for async contexts.

impl<M: CompletionModel + Send + Sync> LlmRule<M> {
    /// Get the rule ID.
    pub fn id(&self) -> RuleId {
        self.rule_id.clone()
    }
}

// =============================================================================
// Async Automaton Extensions
// =============================================================================

/// Result of an async tick.
#[derive(Debug, Clone)]
pub struct AsyncTickResult {
    /// Tick number.
    pub tick: u64,
    /// Number of successful transitions.
    pub transitions: usize,
    /// Number of failed LLM calls.
    pub errors: usize,
    /// Error messages for failed nodes.
    pub error_details: Vec<(vibe_graph_core::NodeId, String)>,
    /// Duration of the tick.
    pub duration: std::time::Duration,
}

/// Extension trait for async automaton operations.
#[async_trait::async_trait]
pub trait AsyncAutomaton {
    /// Execute a single tick asynchronously using LLM rules.
    async fn tick_async<M: CompletionModel + Send + Sync>(
        &mut self,
        llm_rule: &LlmRule<M>,
    ) -> AutomatonResult<AsyncTickResult>;

    /// Run until stable or max ticks, using LLM rules.
    async fn run_async<M: CompletionModel + Send + Sync>(
        &mut self,
        llm_rule: &LlmRule<M>,
        max_ticks: usize,
    ) -> AutomatonResult<Vec<AsyncTickResult>>;
}

// Implementation for GraphAutomaton
use crate::automaton::GraphAutomaton;
use crate::temporal::TemporalGraph;

#[async_trait::async_trait]
impl AsyncAutomaton for GraphAutomaton {
    async fn tick_async<M: CompletionModel + Send + Sync>(
        &mut self,
        llm_rule: &LlmRule<M>,
    ) -> AutomatonResult<AsyncTickResult> {
        use std::time::Instant;

        let started = Instant::now();
        let tick = self.tick_count();

        debug!(tick = tick, "async_tick_start");

        let node_ids = self.graph().node_ids();
        let mut transitions = 0;
        let mut errors = 0;
        let mut error_details = Vec::new();

        // Process nodes sequentially (use tick_async_parallel for concurrent processing)
        for node_id in node_ids {
            // Build context for this node
            let ctx = self.build_rule_context(node_id)?;

            // Call LLM
            match llm_rule.infer_async(&ctx).await {
                Ok(output) => {
                    // Convert to StateData
                    let new_state =
                        llm_rule.to_state_data(output.clone(), ctx.state.current_state());

                    // Determine rule ID from output
                    let rule_id = if output.rule.is_empty() {
                        llm_rule.id()
                    } else {
                        RuleId::new(&output.rule)
                    };

                    // Apply transition
                    self.graph_mut()
                        .apply_transition(&node_id, rule_id, new_state)?;
                    transitions += 1;
                }
                Err(e) => {
                    errors += 1;
                    error_details.push((node_id, e.to_string()));
                }
            }
        }

        // Increment tick counter
        self.increment_tick();

        let duration = started.elapsed();

        debug!(
            tick = tick,
            transitions = transitions,
            errors = errors,
            duration_ms = duration.as_millis() as u64,
            "async_tick_complete"
        );

        Ok(AsyncTickResult {
            tick,
            transitions,
            errors,
            error_details,
            duration,
        })
    }

    async fn run_async<M: CompletionModel + Send + Sync>(
        &mut self,
        llm_rule: &LlmRule<M>,
        max_ticks: usize,
    ) -> AutomatonResult<Vec<AsyncTickResult>> {
        let mut results = Vec::with_capacity(max_ticks);

        for _ in 0..max_ticks {
            let result = self.tick_async(llm_rule).await?;

            // Check for stability (no transitions means stable)
            let is_stable = result.transitions == 0 && result.errors == 0;

            results.push(result);

            if is_stable {
                debug!("automaton_stabilized");
                break;
            }
        }

        Ok(results)
    }
}

// =============================================================================
// Distributed/Parallel Tick (like dynamical-system's distributed_step)
// =============================================================================

/// Execute a parallel tick using multiple resolvers.
///
/// This mirrors the `distributed_step` pattern from `dynamical-system`:
/// - Nodes are shuffled and divided into chunks
/// - Each chunk is processed concurrently using different resolvers
/// - Results are collected and applied atomically
///
/// # Arguments
/// * `automaton` - The automaton to evolve
/// * `resolver_pool` - Pool of LLM resolvers for round-robin distribution
/// * `concurrency` - Maximum concurrent LLM calls (defaults to resolver pool size)
pub async fn tick_async_distributed(
    automaton: &mut GraphAutomaton,
    resolver_pool: &ResolverPool,
    concurrency: Option<usize>,
) -> AutomatonResult<AsyncTickResult> {
    use rand::seq::SliceRandom;
    use std::time::Instant;

    let started = Instant::now();
    let tick = automaton.tick_count();
    let concurrency = concurrency.unwrap_or_else(|| resolver_pool.len().max(1));

    debug!(
        tick = tick,
        concurrency = concurrency,
        "distributed_tick_start"
    );

    // Get and shuffle node IDs
    let mut node_ids = automaton.graph().node_ids();
    {
        let mut rng = rand::rng();
        node_ids.shuffle(&mut rng);
    }

    let mut transitions = 0;
    let mut errors = 0;
    let mut error_details = Vec::new();
    let mut updates: Vec<(vibe_graph_core::NodeId, RuleId, StateData)> = Vec::new();

    // Process nodes in chunks for concurrency
    for chunk in node_ids.chunks(concurrency) {
        let mut tasks = Vec::with_capacity(chunk.len());

        for (i, &node_id) in chunk.iter().enumerate() {
            // Get resolver for this task
            let resolver = resolver_pool.get(i % resolver_pool.len()).ok_or_else(|| {
                AutomatonError::GraphConstruction {
                    message: "No resolvers available".to_string(),
                }
            })?;

            // Build context
            let ctx = automaton.build_rule_context(node_id)?;

            // Build prompt (using the same format as LlmRule)
            let system_prompt =
                DEFAULT_SYSTEM_PROMPT.replace("{schema}", NextStateOutput::schema_description());
            let user_message = build_user_message_from_context(&ctx, 4);
            let full_prompt = format!(
                "{}\n\n---\n\nCurrent context:\n{}",
                system_prompt, user_message
            );
            let current_state = ctx.state.current_state().clone();

            // Create agent for this resolver (inside spawn to avoid move issues)
            let resolver_clone = resolver.clone();

            // Spawn task
            tasks.push(tokio::spawn(async move {
                let agent = create_agent_from_resolver(&resolver_clone);
                let result = agent.prompt(full_prompt).await;
                (node_id, result, current_state)
            }));
        }

        // Wait for all tasks in this chunk
        let results = futures::future::join_all(tasks).await;

        for result in results {
            match result {
                Ok((node_id, Ok(response), current_state)) => {
                    // Parse response
                    let cleaned = response
                        .trim()
                        .trim_matches('`')
                        .trim_start_matches("json")
                        .trim_start_matches("JSON")
                        .trim_matches(['`', ' ', '\n', '\r'])
                        .trim();

                    match serde_json::from_str::<NextStateOutput>(cleaned) {
                        Ok(output) => {
                            let new_state = StateData {
                                payload: output.state,
                                activation: output.activation.unwrap_or(current_state.activation),
                                annotations: if let Some(feedback) = output.feedback {
                                    let mut ann = current_state.annotations.clone();
                                    ann.insert("llm:feedback".to_string(), feedback);
                                    ann
                                } else {
                                    current_state.annotations.clone()
                                },
                            };

                            let rule_id = if output.rule.is_empty() {
                                RuleId::new("llm_cognitive_unit")
                            } else {
                                RuleId::new(&output.rule)
                            };

                            updates.push((node_id, rule_id, new_state));
                            transitions += 1;
                        }
                        Err(e) => {
                            errors += 1;
                            error_details.push((node_id, format!("Parse error: {}", e)));
                        }
                    }
                }
                Ok((node_id, Err(e), _)) => {
                    errors += 1;
                    error_details.push((node_id, format!("LLM error: {}", e)));
                }
                Err(e) => {
                    errors += 1;
                    error_details.push((vibe_graph_core::NodeId(0), format!("Task error: {}", e)));
                }
            }
        }
    }

    // Apply all updates atomically
    for (node_id, rule_id, new_state) in updates {
        automaton
            .graph_mut()
            .apply_transition(&node_id, rule_id, new_state)?;
    }

    automaton.increment_tick();

    let duration = started.elapsed();

    debug!(
        tick = tick,
        transitions = transitions,
        errors = errors,
        duration_ms = duration.as_millis() as u64,
        "distributed_tick_complete"
    );

    Ok(AsyncTickResult {
        tick,
        transitions,
        errors,
        error_details,
        duration,
    })
}

/// Run the automaton with distributed LLM processing.
pub async fn run_async_distributed(
    automaton: &mut GraphAutomaton,
    resolver_pool: &ResolverPool,
    max_ticks: usize,
    concurrency: Option<usize>,
) -> AutomatonResult<Vec<AsyncTickResult>> {
    let mut results = Vec::with_capacity(max_ticks);

    for _ in 0..max_ticks {
        let result = tick_async_distributed(automaton, resolver_pool, concurrency).await?;

        let is_stable = result.transitions == 0 && result.errors == 0;
        results.push(result);

        if is_stable {
            debug!("distributed_automaton_stabilized");
            break;
        }
    }

    Ok(results)
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create an OpenAI-compatible Rig client.
pub fn create_openai_client(resolver: &LlmResolver) -> openai::Client {
    openai::Client::from_url(&resolver.api_key, &resolver.api_url)
}

/// Create an agent from a resolver.
pub fn create_agent_from_resolver(resolver: &LlmResolver) -> Agent<openai::CompletionModel> {
    let client = create_openai_client(resolver);
    client.agent(&resolver.model_name).build()
}

/// Build user message from context (standalone helper for distributed processing).
fn build_user_message_from_context(ctx: &RuleContext<'_>, memory_window: usize) -> String {
    // Current state
    let current = ctx.state.current();
    let current_json = json!({
        "rule": current.rule_id.name(),
        "state": current.state.payload.clone(),
        "activation": current.state.activation,
    });

    // Recent history
    let history: Vec<Value> = ctx
        .state
        .recent_history(memory_window)
        .iter()
        .map(|t| {
            json!({
                "rule": t.rule_id.name(),
                "state": t.state.payload.clone(),
                "activation": t.state.activation,
            })
        })
        .collect();

    // Neighbors
    let neighbors: Vec<Value> = ctx
        .neighbors
        .iter()
        .map(|n| {
            let state = n.state.current_state();
            json!({
                "relationship": n.relationship.clone(),
                "rule": n.state.current_rule().name(),
                "state": state.payload.clone(),
                "activation": state.activation,
            })
        })
        .collect();

    // Build full context
    let context = json!({
        "current": current_json,
        "memory": history,
        "neighbors": neighbors,
        "tick": ctx.tick,
    });

    serde_json::to_string_pretty(&context).unwrap_or_else(|_| "{}".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_state_output_parse() {
        let json = r#"{"rule": "test_rule", "state": {"count": 42}, "activation": 0.8}"#;
        let output: NextStateOutput = serde_json::from_str(json).unwrap();

        assert_eq!(output.rule, "test_rule");
        assert_eq!(output.state, json!({"count": 42}));
        assert_eq!(output.activation, Some(0.8));
    }

    #[test]
    fn test_next_state_output_minimal() {
        let json = r#"{"rule": "r", "state": "s"}"#;
        let output: NextStateOutput = serde_json::from_str(json).unwrap();

        assert_eq!(output.rule, "r");
        assert_eq!(output.state, json!("s"));
        assert_eq!(output.activation, None);
        assert_eq!(output.feedback, None);
    }

    #[test]
    fn test_resolver_pool_round_robin() {
        let resolvers = vec![
            LlmResolver::new("url1", "key1", "model1"),
            LlmResolver::new("url2", "key2", "model2"),
            LlmResolver::new("url3", "key3", "model3"),
        ];

        let pool = ResolverPool::new(resolvers);

        assert_eq!(pool.next().unwrap().api_url, "url1");
        assert_eq!(pool.next().unwrap().api_url, "url2");
        assert_eq!(pool.next().unwrap().api_url, "url3");
        assert_eq!(pool.next().unwrap().api_url, "url1"); // wraps around
    }

    #[test]
    fn test_resolver_ollama() {
        let resolver = LlmResolver::ollama("llama3");
        assert_eq!(resolver.api_url, "http://localhost:11434/v1");
        assert_eq!(resolver.model_name, "llama3");
    }
}
