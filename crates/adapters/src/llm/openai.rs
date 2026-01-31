//! OpenAI Responses API adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// OpenAI classifier using the Responses API
pub struct OpenAiClassifier {
    client: Client,
    api_key: SecretString,
    base_url: String,
    config: LlmConfig,
}

impl OpenAiClassifier {
    pub fn new(api_key: SecretString, config: LlmConfig) -> Self {
        Self::with_base_url(api_key, "https://api.openai.com/v1".to_string(), config)
    }

    pub fn with_base_url(api_key: SecretString, base_url: String, config: LlmConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            api_key,
            base_url,
            config,
        }
    }

    async fn call_api(&self, prompt: &str) -> Result<String, ClassifyError> {
        let request = OpenAiRequest {
            model: self.config.model.clone(),
            input: prompt.to_string(),
            instructions: Some(
                "You are a narrative analysis system. Output only valid JSON.".to_string(),
            ),
            temperature: Some(self.config.temperature),
            max_output_tokens: Some(self.config.max_output_tokens),
        };

        let url = format!("{}/responses", self.base_url);

        let response = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
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

        let api_response: OpenAiResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        // Extract text from response
        let text = api_response
            .output
            .into_iter()
            .filter_map(|item| {
                if item.r#type == "message" {
                    item.content.into_iter().find_map(|c| {
                        if c.r#type == "output_text" {
                            Some(c.text)
                        } else {
                            None
                        }
                    })
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
struct OpenAiRequest {
    model: String,
    input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct OpenAiResponse {
    output: Vec<OutputItem>,
}

#[derive(Deserialize)]
struct OutputItem {
    r#type: String,
    #[serde(default)]
    content: Vec<ContentItem>,
}

#[derive(Deserialize)]
struct ContentItem {
    r#type: String,
    #[serde(default)]
    text: String,
}

#[async_trait]
impl Classifier for OpenAiClassifier {
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
                        tracing::warn!(error = %e, "Failed to parse response, will retry");
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

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{SourcePost, TagDefinition};
    use time::OffsetDateTime;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_input() -> ClassifyInput {
        ClassifyInput {
            post: SourcePost {
                id: "123".to_string(),
                text: "Climate change is causing disasters".to_string(),
                author: "testuser".to_string(),
                url: "https://x.com/testuser/status/123".to_string(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            },
            definitions: vec![TagDefinition {
                id: "climate_fear".to_string(),
                title: "Climate Fear".to_string(),
                aliases: vec![],
                short: Some("Fear-based messaging".to_string()),
                content: "Definition content".to_string(),
                file_path: "climate_fear.md".to_string(),
            }],
            max_output_chars: None,
            policy_text: None,
        }
    }

    fn mock_success_response() -> serde_json::Value {
        serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        {
                            "type": "output_text",
                            "text": r#"{"version":"1","summary":"Test summary","tags":[{"id":"climate_fear","confidence":0.85,"rationale":"Test rationale","evidence":["disasters"]}]}"#
                        }
                    ]
                }
            ]
        })
    }

    #[tokio::test]
    async fn test_classify_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/responses"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(mock_success_response()))
            .mount(&mock_server)
            .await;

        let classifier = OpenAiClassifier::with_base_url(
            SecretString::new("test-key".into()),
            mock_server.uri(),
            LlmConfig::default(),
        );

        let result = classifier.classify(sample_input()).await.unwrap();

        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "climate_fear");
    }

    #[tokio::test]
    async fn test_classify_rate_limited() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock_server)
            .await;

        let classifier = OpenAiClassifier::with_base_url(
            SecretString::new("test-key".into()),
            mock_server.uri(),
            LlmConfig {
                retries: 0,
                ..Default::default()
            },
        );

        let result = classifier.classify(sample_input()).await;

        assert!(matches!(result, Err(ClassifyError::RateLimited)));
    }

    #[tokio::test]
    async fn test_classify_api_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal error"))
            .mount(&mock_server)
            .await;

        let classifier = OpenAiClassifier::with_base_url(
            SecretString::new("test-key".into()),
            mock_server.uri(),
            LlmConfig {
                retries: 0,
                ..Default::default()
            },
        );

        let result = classifier.classify(sample_input()).await;

        assert!(matches!(result, Err(ClassifyError::Api(_))));
    }
}
