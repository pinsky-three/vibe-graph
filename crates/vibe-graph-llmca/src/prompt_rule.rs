use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use schemars::{schema_for, JsonSchema};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, warn};
use vibe_graph_core::{Constitution, Vibe};

use crate::{Cell, CellState, CellUpdateRule};

const DEFAULT_MEMORY_WINDOW: usize = 4;

/// Resolver definition for LLM calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResolver {
    pub api_url: String,
    pub api_key: String,
    pub model_name: String,
}

impl LlmResolver {
    /// Load resolver definitions from comma separated environment variables.
    pub fn load_from_env() -> Result<Vec<Self>> {
        let urls = read_env_list("VIBE_GRAPH_LLMCA_API_URLS")
            .or_else(|| read_env_list("OPENAI_API_URL"))
            .unwrap_or_else(|| vec!["http://localhost:11434/v1".to_string()]);
        let models = read_env_list("VIBE_GRAPH_LLMCA_MODELS")
            .or_else(|| read_env_list("OPENAI_MODEL_NAME"))
            .unwrap_or_else(|| vec!["phi3".to_string()]);
        let keys = read_env_list("VIBE_GRAPH_LLMCA_KEYS")
            .or_else(|| read_env_list("OPENAI_API_KEY"))
            .unwrap_or_else(|| vec!["ollama".to_string()]);

        if urls.len() != models.len() || models.len() != keys.len() {
            return Err(anyhow!(
                "resolver env vars must provide the same number of entries"
            ));
        }

        Ok(urls
            .into_iter()
            .zip(models)
            .zip(keys)
            .map(|((api_url, model_name), api_key)| Self {
                api_url,
                api_key,
                model_name,
            })
            .collect())
    }

    /// Load resolvers from a TOML file mirroring the legacy `dynamical-system` layout.
    pub fn load_from_toml<P: AsRef<Path>>(path: P) -> Result<Vec<Self>> {
        #[derive(Deserialize)]
        struct ResolverFile {
            resolvers: Vec<LlmResolver>,
        }

        let raw = fs::read_to_string(path.as_ref()).with_context(|| {
            format!("unable to read resolver file: {}", path.as_ref().display())
        })?;
        let config: ResolverFile = toml::from_str(&raw).context("invalid resolver toml")?;
        Ok(config.resolvers)
    }
}

fn read_env_list(key: &str) -> Option<Vec<String>> {
    env::var(key).ok().map(|raw| {
        raw.split(',')
            .map(|entry| entry.trim().to_string())
            .filter(|entry| !entry.is_empty())
            .collect()
    })
}

/// Prompt program that mirrors the behavior of the original LLMCA lattice.
pub struct PromptProgrammedRule {
    resolvers: Vec<LlmResolver>,
    client: Client,
    cursor: AtomicUsize,
    prompt: PromptTemplate,
    memory_window: usize,
}

impl PromptProgrammedRule {
    /// Build the rule from resolvers. At least one resolver is required.
    pub fn new(resolvers: Vec<LlmResolver>) -> Result<Self> {
        if resolvers.is_empty() {
            return Err(anyhow!("at least one resolver required"));
        }

        // Decision: use the blocking client so the engine can stay synchronous for now.
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            resolvers,
            client,
            cursor: AtomicUsize::new(0),
            prompt: PromptTemplate::default(),
            memory_window: DEFAULT_MEMORY_WINDOW,
        })
    }

    /// Convenience constructor using resolver configuration drawn from the environment.
    pub fn from_env() -> Result<Self> {
        let resolvers = LlmResolver::load_from_env()?;
        Self::new(resolvers)
    }

    /// Override the system prompt used when querying the resolvers.
    pub fn with_prompt(mut self, prompt: PromptTemplate) -> Self {
        self.prompt = prompt;
        self
    }

    /// Configure how much history is surfaced to the model for each call.
    pub fn with_memory_window(mut self, memory_window: usize) -> Self {
        self.memory_window = memory_window.max(1);
        self
    }

    fn predict(
        &self,
        cell: &Cell,
        neighbors: &[Cell],
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> CellState {
        let resolver = self.select_resolver();
        let payload = self.build_user_payload(cell, neighbors, vibes, constitution);

        match self.invoke_model(resolver, &payload) {
            Ok(raw) => self.state_from_response(&raw, cell),
            Err(err) => {
                warn!(
                    target: "llmca::rule",
                    node = cell.node_id.0,
                    model = %resolver.model_name,
                    "llm invocation failed: {err}"
                );
                self.fallback_state(cell, &format!("resolver_error: {err}"))
            }
        }
    }

    fn select_resolver(&self) -> &LlmResolver {
        let idx = self.cursor.fetch_add(1, Ordering::Relaxed);
        let index = idx % self.resolvers.len();
        &self.resolvers[index]
    }

    fn invoke_model(&self, resolver: &LlmResolver, payload: &str) -> Result<String> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if !resolver.api_key.is_empty() {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", resolver.api_key))
                    .context("invalid api key header")?,
            );
        }

        let url = format!(
            "{}/chat/completions",
            resolver.api_url.trim_end_matches('/')
        );
        let body = json!({
            "model": resolver.model_name,
            "messages": [
                {"role": "system", "content": self.prompt.system_prompt.clone()},
                {"role": "user", "content": payload}
            ]
        });

        let response = self
            .client
            .post(url)
            .headers(headers)
            .json(&body)
            .send()
            .with_context(|| "llm call failed".to_string())?
            .error_for_status()
            .context("llm http error")?;

        let completion: ChatCompletionResponse =
            response.json().context("malformed llm response")?;
        let content = completion
            .choices
            .first()
            .ok_or_else(|| anyhow!("llm response missing choices"))?
            .message
            .content
            .clone();
        debug!(target: "llmca::rule", chars = content.len(), "llm response received");
        Ok(content)
    }

    fn state_from_response(&self, raw: &str, cell: &Cell) -> CellState {
        let cleaned = Self::scrub_response(raw);
        match serde_json::from_str::<AutomatonStateProposal>(cleaned) {
            Ok(proposal) => {
                let mut next = cell.state.clone();
                next.payload = proposal.payload;
                if let Some(activation) = proposal.activation {
                    next.activation = activation;
                }
                if !proposal.annotations.is_empty() {
                    next.annotations.extend(proposal.annotations);
                }
                next.last_updated = SystemTime::now();
                next
            }
            Err(err) => {
                warn!(
                    target: "llmca::rule",
                    node = cell.node_id.0,
                    "failed to parse llm response: {err}"
                );
                self.fallback_state(cell, cleaned)
            }
        }
    }

    fn fallback_state(&self, cell: &Cell, diagnostic: &str) -> CellState {
        let mut next = cell.state.clone();
        next.payload = Value::String(diagnostic.to_string());
        next.annotations
            .insert("llmca:error".into(), diagnostic.to_string());
        next.last_updated = SystemTime::now();
        next
    }

    fn build_user_payload(
        &self,
        cell: &Cell,
        neighbors: &[Cell],
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> String {
        let history = cell
            .history
            .iter()
            .rev()
            .take(self.memory_window)
            .map(|state| {
                json!({
                    "payload": state.payload.clone(),
                    "activation": state.activation,
                    "annotations": state.annotations.clone(),
                })
            })
            .collect::<Vec<_>>();

        let neighbors_json = neighbors
            .iter()
            .map(|neighbor| {
                json!({
                    "node_id": neighbor.node_id.0,
                    "payload": neighbor.state.payload.clone(),
                    "activation": neighbor.state.activation,
                    "annotations": neighbor.state.annotations.clone(),
                })
            })
            .collect::<Vec<_>>();

        let vibes_json = vibes
            .iter()
            .map(|v| {
                json!({
                    "id": v.id.clone(),
                    "title": v.title.clone(),
                    "description": v.description.clone(),
                    "targets": v.targets.iter().map(|t| t.0).collect::<Vec<_>>(),
                    "metadata": v.metadata.clone(),
                })
            })
            .collect::<Vec<_>>();

        let constitution_json = json!({
            "name": constitution.name.clone(),
            "version": constitution.version.clone(),
            "description": constitution.description.clone(),
            "policies": constitution.policies.clone(),
        });

        json!({
            "node_id": cell.node_id.0,
            "current_state": {
                "payload": cell.state.payload.clone(),
                "activation": cell.state.activation,
                "annotations": cell.state.annotations.clone(),
            },
            "recent_history": history,
            "neighbors": neighbors_json,
            "vibes": vibes_json,
            "constitution": constitution_json,
        })
        .to_string()
    }

    fn scrub_response(raw: &str) -> &str {
        raw.trim()
            .trim_matches('`')
            .trim_start_matches("json")
            .trim_matches(['`', ' ', '\n'])
    }
}

impl CellUpdateRule for PromptProgrammedRule {
    fn update(
        &self,
        cell: &Cell,
        neighbors: &[Cell],
        vibes: &[Vibe],
        constitution: &Constitution,
    ) -> CellState {
        self.predict(cell, neighbors, vibes, constitution)
    }
}

/// Container for the prompt template so callers can override copy if needed.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub system_prompt: String,
}

impl PromptTemplate {
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: prompt.into(),
        }
    }
}

impl Default for PromptTemplate {
    fn default() -> Self {
        let schema = serde_json::to_string_pretty(&schema_for!(AutomatonStateProposal))
            .unwrap_or_else(|_| "{}".into());
        let prompt = ["You are a Large Language Model Cellular Automaton (LLMCA) unit.",
            "Return the next state for your node as JSON matching this schema:",
            &format!("```json\n{}\n```", schema),
            "Inputs include your current payload, a short memory window, neighbor summaries, current vibes, and the active constitution.",
            "Reason about the neighbors to coordinate, keep payloads concise, and never wrap the JSON in explanations."]
        .join("\n\n");
        Self::new(prompt)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AutomatonStateProposal {
    payload: Value,
    #[serde(default)]
    activation: Option<f32>,
    #[serde(default)]
    annotations: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

#[derive(Debug, Deserialize)]
struct Message {
    content: String,
}
