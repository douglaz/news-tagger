//! Anthropic Claude API adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// Anthropic classifier
pub struct AnthropicClassifier {
    client: Client,
    api_key: SecretString,
    config: LlmConfig,
}

impl AnthropicClassifier {
    pub fn new(api_key: SecretString, config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key,
            config,
        }
    }

    async fn call_api(&self, prompt: &str) -> Result<String, ClassifyError> {
        let request = AnthropicRequest {
            model: self.config.model.clone(),
            max_tokens: self.config.max_output_tokens,
            messages: vec![Message {
                role: "user".to_string(),
                content: prompt.to_string(),
            }],
            system: Some(
                "You are a narrative analysis system. Output only valid JSON.".to_string(),
            ),
            temperature: Some(self.config.temperature),
        };

        let response = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", self.api_key.expose_secret())
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ClassifyError::Timeout
                } else {
                    ClassifyError::Api(e.to_string())
                }
            })?;

        if response.status() == 429 {
            return Err(ClassifyError::RateLimited);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ClassifyError::Api(format!(
                "API returned {}: {}",
                status, body
            )));
        }

        let api_response: AnthropicResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        let text = api_response
            .content
            .into_iter()
            .filter_map(|c| {
                if c.r#type == "text" {
                    Some(c.text)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(ClassifyError::InvalidFormat("Empty response".to_string()));
        }

        Ok(text)
    }
}

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    r#type: String,
    #[serde(default)]
    text: String,
}

#[async_trait]
impl Classifier for AnthropicClassifier {
    async fn classify(&self, input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
        let prompt = build_classification_prompt(
            &input.post.text,
            &input.post.author,
            &input.definitions,
            input.policy_text.as_deref(),
        );

        let mut last_error = None;
        for attempt in 0..=self.config.retries {
            if attempt > 0 {
                tracing::warn!(attempt = attempt, "Retrying classification");
                tokio::time::sleep(Duration::from_millis(500 * 2_u64.pow(attempt))).await;
            }

            match self.call_api(&prompt).await {
                Ok(response_text) => match parse_classification_response(&response_text) {
                    Ok(output) => return Ok(output),
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to parse response");
                        last_error = Some(ClassifyError::InvalidFormat(e));
                    }
                },
                Err(ClassifyError::RateLimited) => {
                    return Err(ClassifyError::RateLimited);
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ClassifyError::Api("Unknown error".to_string())))
    }
}
