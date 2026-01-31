//! Nostr publishing adapter

use async_trait::async_trait;
use news_tagger_domain::{PublishError, PublishResult, Publisher, RenderedPost};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;
use time::OffsetDateTime;

/// Nostr publisher for creating notes
pub struct NostrPublisher {
    client: Client,
    secret_key: SecretString,
    relays: Vec<String>,
    enabled: bool,
}

impl NostrPublisher {
    pub fn new(secret_key: SecretString, relays: Vec<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            client,
            secret_key,
            relays,
            enabled: true,
        }
    }

    /// Create a disabled publisher (for testing/dry-run)
    pub fn disabled() -> Self {
        Self {
            client: Client::new(),
            secret_key: SecretString::new("".into()),
            relays: vec![],
            enabled: false,
        }
    }

    /// Generate a Nostr event (NIP-01)
    fn create_event(&self, content: &str) -> NostrEvent {
        let created_at = OffsetDateTime::now_utc().unix_timestamp();

        // For a real implementation, we'd need proper secp256k1 signing
        // This is a placeholder that shows the structure
        let pubkey = self.derive_pubkey();

        let event_data = format!(
            "[0,\"{}\",{},1,[],\"{}\"]",
            pubkey,
            created_at,
            content.replace('\"', "\\\"").replace('\n', "\\n")
        );

        let mut hasher = Sha256::new();
        hasher.update(event_data.as_bytes());
        let id = format!("{:x}", hasher.finalize());

        // Placeholder signature - real implementation needs secp256k1
        let sig = "placeholder_signature".to_string();

        NostrEvent {
            id,
            pubkey,
            created_at,
            kind: 1, // Text note
            tags: vec![],
            content: content.to_string(),
            sig,
        }
    }

    /// Derive public key from secret key (placeholder)
    fn derive_pubkey(&self) -> String {
        // Real implementation would use secp256k1
        let mut hasher = Sha256::new();
        hasher.update(self.secret_key.expose_secret().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Publish event to a relay via HTTP (NIP-86 or relay-specific HTTP endpoint)
    async fn publish_to_relay(&self, relay: &str, event: &NostrEvent) -> Result<(), PublishError> {
        // Most relays use WebSocket, but some support HTTP
        // This is a simplified implementation
        let url = if relay.starts_with("wss://") {
            // Convert to HTTP endpoint if relay supports it
            relay.replace("wss://", "https://")
        } else if relay.starts_with("ws://") {
            relay.replace("ws://", "http://")
        } else {
            relay.to_string()
        };

        let request = vec![
            "EVENT".to_string(),
            serde_json::to_string(event).map_err(|e| PublishError::Api(e.to_string()))?,
        ];

        let response = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await
            .map_err(|e| PublishError::Api(format!("Failed to connect to relay: {}", e)))?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(PublishError::Api(format!("Relay rejected event: {}", body)));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NostrEvent {
    id: String,
    pubkey: String,
    created_at: i64,
    kind: u32,
    tags: Vec<Vec<String>>,
    content: String,
    sig: String,
}

#[async_trait]
impl Publisher for NostrPublisher {
    async fn publish(&self, post: &RenderedPost) -> Result<PublishResult, PublishError> {
        if !self.enabled {
            return Err(PublishError::Api("Publisher is disabled".to_string()));
        }

        if self.relays.is_empty() {
            return Err(PublishError::Api("No relays configured".to_string()));
        }

        let event = self.create_event(&post.text);
        let event_id = event.id.clone();

        // Try to publish to at least one relay
        let mut last_error = None;
        let mut success_count = 0;

        for relay in &self.relays {
            match self.publish_to_relay(relay, &event).await {
                Ok(()) => {
                    tracing::info!(relay = %relay, event_id = %event_id, "Published to Nostr relay");
                    success_count += 1;
                }
                Err(e) => {
                    tracing::warn!(relay = %relay, error = %e, "Failed to publish to relay");
                    last_error = Some(e);
                }
            }
        }

        if success_count == 0 {
            return Err(last_error.unwrap_or_else(|| {
                PublishError::Api("Failed to publish to any relay".to_string())
            }));
        }

        Ok(PublishResult {
            id: event_id,
            url: None, // Nostr doesn't have a canonical URL
        })
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    fn platform(&self) -> &'static str {
        "nostr"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_post() -> RenderedPost {
        RenderedPost {
            text: "Narrative analysis of @user\n\nTags: test_tag (0.85)\n\nOriginal: https://x.com/user/status/123".to_string(),
            source_post_id: "123".to_string(),
            source_post_url: "https://x.com/user/status/123".to_string(),
        }
    }

    #[tokio::test]
    async fn test_disabled_publisher() {
        let publisher = NostrPublisher::disabled();

        assert!(!publisher.is_enabled());
        assert_eq!(publisher.platform(), "nostr");

        let result = publisher.publish(&sample_post()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_no_relays_configured() {
        let publisher = NostrPublisher::new(SecretString::new("test-secret-key".into()), vec![]);

        let result = publisher.publish(&sample_post()).await;
        assert!(matches!(result, Err(PublishError::Api(_))));
    }

    #[tokio::test]
    async fn test_publish_success() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "ok"
            })))
            .mount(&mock_server)
            .await;

        let publisher = NostrPublisher::new(
            SecretString::new("test-secret-key".into()),
            vec![mock_server.uri()],
        );

        let result = publisher.publish(&sample_post()).await.unwrap();

        assert!(!result.id.is_empty());
    }

    #[tokio::test]
    async fn test_create_event() {
        let publisher = NostrPublisher::new(SecretString::new("test-secret-key".into()), vec![]);

        let event = publisher.create_event("Test content");

        assert!(!event.id.is_empty());
        assert!(!event.pubkey.is_empty());
        assert_eq!(event.kind, 1);
        assert_eq!(event.content, "Test content");
    }
}
