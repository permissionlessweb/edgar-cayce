use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct LlmClient {
    client: reqwest::Client,
    base_url: String,
    model: String,
    sub_model: String,
    api_key: Option<String>,
}

impl LlmClient {
    pub fn from_env() -> Result<Self> {
        let base_url = dotenv::var("LLM_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:1234/v1".to_string());
        let model =
            dotenv::var("LLM_MODEL").unwrap_or_else(|_| "qwen/qwen3-8b".to_string());
        let sub_model =
            dotenv::var("LLM_SUB_MODEL").unwrap_or_else(|_| model.clone());
        let api_key = dotenv::var("LLM_API_KEY").ok().filter(|k| !k.is_empty());

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            base_url,
            model,
            sub_model,
            api_key,
        })
    }

    /// Resolve the chat completions endpoint from the base URL.
    fn endpoint(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        if base.ends_with("/chat/completions") {
            base.to_string()
        } else if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    /// Non-streaming chat completion.
    pub async fn chat(
        &self,
        messages: &[Message],
        model_override: Option<&str>,
    ) -> Result<String> {
        let model = model_override.unwrap_or(&self.model);
        let body = serde_json::json!({
            "model": model,
            "messages": messages,
            "temperature": 0.3,
            "max_tokens": 2048,
        });

        let mut req = self.client.post(self.endpoint()).json(&body);
        if let Some(key) = &self.api_key {
            req = req.header("Authorization", format!("Bearer {}", key));
        }

        let resp = req.send().await.context("LLM request failed")?;
        let text = resp.text().await.context("Failed to read LLM response")?;
        let json: serde_json::Value =
            serde_json::from_str(&text).context("Failed to parse LLM JSON")?;

        // Extract content from choices[0].message.content (handle null)
        let content = json["choices"]
            .get(0)
            .and_then(|c| c["message"]["content"].as_str())
            .unwrap_or("")
            .to_string();

        Ok(content)
    }

    /// Sub-LLM query using the sub_model.
    pub async fn sub_query(&self, prompt: &str) -> Result<String> {
        let messages = vec![Message {
            role: "user".to_string(),
            content: prompt.to_string(),
        }];
        self.chat(&messages, Some(&self.sub_model.clone())).await
    }
}
