//! Stub classifier for testing and offline mode

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput, TagMatch};

/// Stub classifier that returns configurable responses
pub struct StubClassifier {
    response: Option<ClassifyOutput>,
    error: Option<ClassifyError>,
}

impl StubClassifier {
    /// Create a stub that returns a successful empty classification
    pub fn empty() -> Self {
        Self {
            response: Some(ClassifyOutput::new(
                "Stub classification - no tags detected".to_string(),
                vec![],
            )),
            error: None,
        }
    }

    /// Create a stub that returns a specific response
    pub fn with_response(response: ClassifyOutput) -> Self {
        Self {
            response: Some(response),
            error: None,
        }
    }

    /// Create a stub that always returns an error
    pub fn with_error(error: ClassifyError) -> Self {
        Self {
            response: None,
            error: Some(error),
        }
    }

    /// Create a stub that echoes back detected keywords as tags
    pub fn echo() -> Self {
        Self {
            response: None,
            error: None,
        }
    }
}

impl Default for StubClassifier {
    fn default() -> Self {
        Self::empty()
    }
}

#[async_trait]
impl Classifier for StubClassifier {
    async fn classify(&self, input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
        // Return configured error if set
        if let Some(ref error) = self.error {
            return Err(match error {
                ClassifyError::Api(msg) => ClassifyError::Api(msg.clone()),
                ClassifyError::InvalidFormat(msg) => ClassifyError::InvalidFormat(msg.clone()),
                ClassifyError::RateLimited => ClassifyError::RateLimited,
                ClassifyError::Timeout => ClassifyError::Timeout,
                ClassifyError::Config(msg) => ClassifyError::Config(msg.clone()),
            });
        }

        // Return configured response if set
        if let Some(ref response) = self.response {
            return Ok(response.clone());
        }

        // Echo mode: find keywords in post that match definition IDs
        let post_lower = input.post.text.to_lowercase();
        let tags: Vec<TagMatch> = input
            .definitions
            .iter()
            .filter_map(|def| {
                // Check if definition ID or title appears in post
                let id_match = post_lower.contains(&def.id.replace('_', " "));
                let title_match = post_lower.contains(&def.title.to_lowercase());

                if id_match || title_match {
                    Some(TagMatch {
                        id: def.id.clone(),
                        confidence: 0.75,
                        rationale: format!("Stub: keyword '{}' detected in post", def.id),
                        evidence: vec![format!(
                            "...{}...",
                            &input.post.text[..50.min(input.post.text.len())]
                        )],
                    })
                } else {
                    None
                }
            })
            .collect();

        Ok(ClassifyOutput::new(
            format!("Stub classification of post by {}", input.post.author),
            tags,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{SourcePost, TagDefinition};
    use time::OffsetDateTime;

    fn sample_input() -> ClassifyInput {
        ClassifyInput {
            post: SourcePost {
                id: "123".to_string(),
                text: "This is about climate fear and economic concerns".to_string(),
                author: "testuser".to_string(),
                url: "https://x.com/testuser/status/123".to_string(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            },
            definitions: vec![
                TagDefinition {
                    id: "climate_fear".to_string(),
                    title: "Climate Fear".to_string(),
                    aliases: vec![],
                    short: None,
                    content: "Definition".to_string(),
                    file_path: "climate_fear.md".to_string(),
                },
                TagDefinition {
                    id: "unrelated_tag".to_string(),
                    title: "Unrelated".to_string(),
                    aliases: vec![],
                    short: None,
                    content: "Definition".to_string(),
                    file_path: "unrelated.md".to_string(),
                },
            ],
            max_output_chars: None,
            policy_text: None,
        }
    }

    #[tokio::test]
    async fn test_empty_stub() {
        let classifier = StubClassifier::empty();
        let result = classifier.classify(sample_input()).await.unwrap();

        assert!(result.tags.is_empty());
    }

    #[tokio::test]
    async fn test_configured_response() {
        let expected = ClassifyOutput::new(
            "Custom summary".to_string(),
            vec![TagMatch {
                id: "custom".to_string(),
                confidence: 0.99,
                rationale: "Custom rationale".to_string(),
                evidence: vec!["evidence".to_string()],
            }],
        );

        let classifier = StubClassifier::with_response(expected.clone());
        let result = classifier.classify(sample_input()).await.unwrap();

        assert_eq!(result.summary, expected.summary);
        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "custom");
    }

    #[tokio::test]
    async fn test_error_stub() {
        let classifier = StubClassifier::with_error(ClassifyError::Timeout);
        let result = classifier.classify(sample_input()).await;

        assert!(matches!(result, Err(ClassifyError::Timeout)));
    }

    #[tokio::test]
    async fn test_echo_stub() {
        let classifier = StubClassifier::echo();
        let result = classifier.classify(sample_input()).await.unwrap();

        // Should detect "climate fear" in the post text
        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "climate_fear");
    }
}
