//! Ollama local LLM adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// Ollama classifier for local LLMs
pub struct OllamaClassifier {
    client: Client,
    base_url: String,
    config: LlmConfig,
}

impl OllamaClassifier {
    pub fn new(config: LlmConfig) -> Self {
        Self::with_base_url("http://localhost:11434".to_string(), config)
    }

    pub fn with_base_url(base_url: String, config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            base_url,
            config,
        }
    }

    async fn call_api(&self, prompt: &str) -> Result<String, ClassifyError> {
        let request = OllamaRequest {
            model: self.config.model.clone(),
            prompt: prompt.to_string(),
            system: Some(
                "You are a narrative analysis system. Output only valid JSON.".to_string(),
            ),
            stream: false,
            options: Some(OllamaOptions {
                temperature: Some(self.config.temperature),
                num_predict: Some(self.config.max_output_tokens as i32),
            }),
        };

        let url = format!("{}/api/generate", self.base_url);

        let response = self
            .client
            .post(&url)
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

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ClassifyError::Api(format!(
                "API returned {}: {}",
                status, body
            )));
        }

        let api_response: OllamaResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        if api_response.response.is_empty() {
            return Err(ClassifyError::InvalidFormat("Empty response".to_string()));
        }

        Ok(api_response.response)
    }
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<i32>,
}

#[derive(Deserialize)]
struct OllamaResponse {
    response: String,
}

#[async_trait]
impl Classifier for OllamaClassifier {
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
                Err(e) => {
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ClassifyError::Api("Unknown error".to_string())))
    }
}
