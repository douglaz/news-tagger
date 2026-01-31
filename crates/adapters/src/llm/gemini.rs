//! Google Gemini API adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// Gemini classifier
pub struct GeminiClassifier {
    client: Client,
    api_key: SecretString,
    config: LlmConfig,
}

impl GeminiClassifier {
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
        let request = GeminiRequest {
            contents: vec![Content {
                parts: vec![Part {
                    text: prompt.to_string(),
                }],
            }],
            generation_config: Some(GenerationConfig {
                temperature: Some(self.config.temperature),
                max_output_tokens: Some(self.config.max_output_tokens),
            }),
            system_instruction: Some(SystemInstruction {
                parts: vec![Part {
                    text: "You are a narrative analysis system. Output only valid JSON."
                        .to_string(),
                }],
            }),
        };

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.config.model,
            self.api_key.expose_secret()
        );

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

        let api_response: GeminiResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        let text = api_response
            .candidates
            .into_iter()
            .flat_map(|c| c.content.parts)
            .map(|p| p.text)
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            return Err(ClassifyError::InvalidFormat("Empty response".to_string()));
        }

        Ok(text)
    }
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<Content>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "generationConfig")]
    generation_config: Option<GenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "systemInstruction")]
    system_instruction: Option<SystemInstruction>,
}

#[derive(Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Serialize, Deserialize)]
struct Part {
    text: String,
}

#[derive(Serialize)]
struct GenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "maxOutputTokens")]
    max_output_tokens: Option<u32>,
}

#[derive(Serialize)]
struct SystemInstruction {
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<Candidate>,
}

#[derive(Deserialize)]
struct Candidate {
    content: ResponseContent,
}

#[derive(Deserialize)]
struct ResponseContent {
    parts: Vec<Part>,
}

#[async_trait]
impl Classifier for GeminiClassifier {
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
