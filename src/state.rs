use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::docs::DocumentStore;
use crate::llm::LlmClient;
use crate::rlm::RlmEngine;

/// Configurable RLM parameters (admins can modify at runtime).
pub struct RlmConfig {
    pub min_code_executions: u32,
    pub max_iterations: u32,
    pub min_answer_len: usize,
}

impl Default for RlmConfig {
    fn default() -> Self {
        Self {
            min_code_executions: 3,
            max_iterations: 15,
            min_answer_len: 150,
        }
    }
}

pub struct AppState {
    pub store: Arc<DocumentStore>,
    pub llm: Arc<LlmClient>,
    pub rlm: Arc<RlmEngine>,
    pub admin_ids: HashSet<u64>,
    pub rlm_config: Arc<RwLock<RlmConfig>>,
}

impl AppState {
    pub fn is_admin(&self, user_id: u64) -> bool {
        self.admin_ids.contains(&user_id)
    }
}

pub type Context<'a> = poise::Context<'a, AppState, anyhow::Error>;
