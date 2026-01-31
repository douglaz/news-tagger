//! OpenCode server API adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::sync::Mutex;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// OpenCode classifier that uses a stateful session over HTTP.
pub struct OpenCodeClassifier {
    client: Client,
    base_url: String,
    config: LlmConfig,
    provider_id: Option<String>,
    model_id: Option<String>,
    session_id: Mutex<Option<String>>,
}

impl OpenCodeClassifier {
    pub fn new(
        base_url: String,
        provider_id: Option<String>,
        model_id: Option<String>,
        session_id: Option<String>,
        config: LlmConfig,
    ) -> Result<Self, ClassifyError> {
        if base_url.trim().is_empty() {
            return Err(ClassifyError::Config(
                "opencode base_url is required".to_string(),
            ));
        }

        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| ClassifyError::Api(e.to_string()))?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            config,
            provider_id: provider_id.filter(|v| !v.trim().is_empty()),
            model_id: model_id.filter(|v| !v.trim().is_empty()),
            session_id: Mutex::new(session_id.filter(|v| !v.trim().is_empty())),
        })
    }

    async fn ensure_session(&self) -> Result<String, ClassifyError> {
        let mut guard = self.session_id.lock().await;
        if let Some(id) = guard.as_ref() {
            return Ok(id.clone());
        }

        let url = format!("{}/session", self.base_url);
        let request = SessionCreateRequest {
            title: Some("news-tagger".to_string()),
        };

        let response = self
            .client
            .post(&url)
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
                "OpenCode session create failed {}: {}",
                status, body
            )));
        }

        let created: SessionCreateResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        if created.id.is_empty() {
            return Err(ClassifyError::InvalidFormat(
                "OpenCode session id missing".to_string(),
            ));
        }

        *guard = Some(created.id.clone());
        Ok(created.id)
    }

    async fn prompt_session(
        &self,
        session_id: &str,
        prompt: &str,
    ) -> Result<String, ClassifyError> {
        let url = format!("{}/session/{}/message", self.base_url, session_id);
        let model = match (&self.provider_id, &self.model_id) {
            (Some(provider_id), Some(model_id)) => Some(ModelRef {
                provider_id: provider_id.clone(),
                model_id: model_id.clone(),
            }),
            _ => None,
        };
        let request = SessionPromptRequest {
            model,
            system: Some(
                "You are a narrative analysis system. Output only valid JSON.".to_string(),
            ),
            parts: vec![PartInput::text(prompt.to_string())],
        };

        let response = self
            .client
            .post(&url)
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
                "OpenCode prompt failed {}: {}",
                status, body
            )));
        }

        let prompt_response: SessionPromptResponse = response
            .json()
            .await
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;

        let text = prompt_response
            .parts
            .into_iter()
            .filter(|part| part.kind == "text")
            .filter_map(|part| part.text)
            .collect::<Vec<_>>()
            .join("");

        if text.trim().is_empty() {
            return Err(ClassifyError::InvalidFormat(
                "OpenCode response missing text".to_string(),
            ));
        }

        Ok(text)
    }
}

#[async_trait]
impl Classifier for OpenCodeClassifier {
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

            let session_id = self.ensure_session().await?;
            match self.prompt_session(&session_id, &prompt).await {
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

#[derive(Serialize)]
struct SessionCreateRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

#[derive(Deserialize)]
struct SessionCreateResponse {
    id: String,
}

#[derive(Serialize)]
struct SessionPromptRequest {
    #[serde(skip_serializing_if = "Option::is_none", rename = "model")]
    model: Option<ModelRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    parts: Vec<PartInput>,
}

#[derive(Serialize, Clone)]
struct ModelRef {
    #[serde(rename = "providerID")]
    provider_id: String,
    #[serde(rename = "modelID")]
    model_id: String,
}

#[derive(Serialize)]
struct PartInput {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

impl PartInput {
    fn text(text: String) -> Self {
        Self {
            kind: "text".to_string(),
            text,
        }
    }
}

#[derive(Deserialize)]
struct SessionPromptResponse {
    parts: Vec<PartOutput>,
}

#[derive(Deserialize)]
struct PartOutput {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{ClassifyInput, SourcePost, TagDefinition};
    use time::OffsetDateTime;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_input() -> ClassifyInput {
        ClassifyInput {
            post: SourcePost {
                id: "1".to_string(),
                text: "Urgent warning".to_string(),
                author: "tester".to_string(),
                url: String::new(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            },
            definitions: vec![TagDefinition {
                id: "fear_narrative".to_string(),
                title: "Fear".to_string(),
                short: None,
                aliases: vec![],
                content: "fear".to_string(),
                file_path: "fear.md".to_string(),
            }],
            max_output_chars: None,
            policy_text: None,
        }
    }

    #[tokio::test]
    async fn test_opencode_classify_creates_session() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/session"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ses_test"
            })))
            .mount(&server)
            .await;

        let prompt_response = serde_json::json!({
            "parts": [
                { "type": "text", "text": "{\"version\":\"1\",\"summary\":\"ok\",\"tags\":[]}" }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/session/ses_test/message"))
            .respond_with(ResponseTemplate::new(200).set_body_json(prompt_response))
            .mount(&server)
            .await;

        let classifier = OpenCodeClassifier::new(
            server.uri(),
            None,
            None,
            None,
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        )
        .unwrap();

        let output = classifier.classify(make_input()).await.unwrap();
        assert_eq!(output.version, "1");
        assert!(output.tags.is_empty());
    }

    #[tokio::test]
    async fn test_opencode_classify_uses_existing_session() {
        let server = MockServer::start().await;

        let prompt_response = serde_json::json!({
            "parts": [
                { "type": "text", "text": "{\"version\":\"1\",\"summary\":\"ok\",\"tags\":[]}" }
            ]
        });

        Mock::given(method("POST"))
            .and(path("/session/ses_existing/message"))
            .and(body_string_contains("Urgent warning"))
            .and(body_string_contains("Output only valid JSON."))
            .respond_with(ResponseTemplate::new(200).set_body_json(prompt_response))
            .mount(&server)
            .await;

        let classifier = OpenCodeClassifier::new(
            server.uri(),
            None,
            None,
            Some("ses_existing".to_string()),
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        )
        .unwrap();

        let output = classifier.classify(make_input()).await.unwrap();
        assert_eq!(output.version, "1");
    }
}
