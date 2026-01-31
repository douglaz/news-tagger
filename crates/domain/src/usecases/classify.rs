//! Classification use case

use crate::{
    model::{ClassifyInput, ClassifyOutput, SourcePost, TagDefinition},
    ports::{Classifier, ClassifyError},
};

/// Configuration for the classify use case
#[derive(Debug, Clone)]
pub struct ClassifyConfig {
    /// Maximum definitions to include (None = include all)
    pub prefilter_top_k: Option<usize>,
    /// Policy/guardrails text to include in prompt
    pub policy_text: Option<String>,
    /// Maximum output characters
    pub max_output_chars: Option<usize>,
}

impl Default for ClassifyConfig {
    fn default() -> Self {
        Self {
            prefilter_top_k: Some(12),
            policy_text: None,
            max_output_chars: None,
        }
    }
}

/// Use case for classifying posts
pub struct ClassifyUseCase<C> {
    classifier: C,
    config: ClassifyConfig,
}

impl<C: Classifier> ClassifyUseCase<C> {
    pub fn new(classifier: C, config: ClassifyConfig) -> Self {
        Self { classifier, config }
    }

    /// Classify a post against the given definitions
    pub async fn classify(
        &self,
        post: &SourcePost,
        definitions: &[TagDefinition],
    ) -> Result<ClassifyOutput, ClassifyError> {
        let selected_definitions = self.select_definitions(post, definitions);

        tracing::info!(
            post_id = %post.id,
            definitions_count = selected_definitions.len(),
            "Classifying post"
        );

        let input = ClassifyInput {
            post: post.clone(),
            definitions: selected_definitions,
            max_output_chars: self.config.max_output_chars,
            policy_text: self.config.policy_text.clone(),
        };

        self.classifier.classify(input).await
    }

    /// Select definitions to include based on prefilter config
    fn select_definitions(
        &self,
        post: &SourcePost,
        definitions: &[TagDefinition],
    ) -> Vec<TagDefinition> {
        match self.config.prefilter_top_k {
            Some(k) if k < definitions.len() => {
                // Simple keyword-based prefilter
                let mut scored: Vec<_> = definitions
                    .iter()
                    .map(|def| {
                        let score = self.compute_relevance_score(post, def);
                        (def.clone(), score)
                    })
                    .collect();

                scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                scored.truncate(k);

                let selected: Vec<_> = scored.into_iter().map(|(def, _)| def).collect();

                tracing::debug!(
                    selected_ids = ?selected.iter().map(|d| &d.id).collect::<Vec<_>>(),
                    "Prefiltered definitions"
                );

                selected
            }
            _ => definitions.to_vec(),
        }
    }

    /// Compute a simple relevance score based on keyword overlap
    fn compute_relevance_score(&self, post: &SourcePost, definition: &TagDefinition) -> f64 {
        let post_lower = post.text.to_lowercase();

        // Check title
        let title_match = if post_lower.contains(&definition.title.to_lowercase()) {
            1.0
        } else {
            0.0
        };

        // Check aliases
        let alias_match = definition
            .aliases
            .iter()
            .filter(|alias| post_lower.contains(&alias.to_lowercase()))
            .count() as f64
            * 0.5;

        // Check short description words
        let short_match = definition
            .short
            .as_ref()
            .map(|s| {
                s.split_whitespace()
                    .filter(|word| word.len() > 3)
                    .filter(|word| post_lower.contains(&word.to_lowercase()))
                    .count() as f64
                    * 0.1
            })
            .unwrap_or(0.0);

        // Check content words (lighter weight to avoid huge docs dominating)
        let content_match = definition
            .content
            .split_whitespace()
            .filter(|word| word.len() > 4)
            .filter(|word| post_lower.contains(&word.to_lowercase()))
            .count() as f64
            * 0.03;

        title_match + alias_match + short_match + content_match
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TagMatch;
    use async_trait::async_trait;
    use time::OffsetDateTime;

    struct FakeClassifier {
        response: ClassifyOutput,
    }

    #[async_trait]
    impl Classifier for FakeClassifier {
        async fn classify(&self, _input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
            Ok(self.response.clone())
        }
    }

    fn sample_post() -> SourcePost {
        SourcePost {
            id: "123".to_string(),
            text: "Climate change is causing unprecedented disasters".to_string(),
            author: "testuser".to_string(),
            url: "https://x.com/testuser/status/123".to_string(),
            created_at: OffsetDateTime::now_utc(),
            is_repost: false,
            is_reply: false,
            reply_to_id: None,
        }
    }

    fn sample_definitions() -> Vec<TagDefinition> {
        vec![
            TagDefinition {
                id: "climate_fear".to_string(),
                title: "Climate Fear".to_string(),
                aliases: vec!["climate doom".to_string()],
                short: Some("Fear-based climate messaging".to_string()),
                content: "# Climate Fear\nDefinition content...".to_string(),
                file_path: "climate_fear.md".to_string(),
            },
            TagDefinition {
                id: "economic_control".to_string(),
                title: "Economic Control".to_string(),
                aliases: vec![],
                short: Some("Centralized economic policies".to_string()),
                content: "# Economic Control\nDefinition content...".to_string(),
                file_path: "economic_control.md".to_string(),
            },
        ]
    }

    #[tokio::test]
    async fn test_classify_returns_output() {
        let expected = ClassifyOutput::new(
            "Post discusses climate disasters".to_string(),
            vec![TagMatch {
                id: "climate_fear".to_string(),
                confidence: 0.85,
                rationale: "Uses fear-based framing".to_string(),
                evidence: vec!["unprecedented disasters".to_string()],
            }],
        );

        let classifier = FakeClassifier {
            response: expected.clone(),
        };
        let usecase = ClassifyUseCase::new(classifier, ClassifyConfig::default());

        let result = usecase
            .classify(&sample_post(), &sample_definitions())
            .await
            .unwrap();

        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "climate_fear");
    }

    #[tokio::test]
    async fn test_prefilter_limits_definitions() {
        let expected = ClassifyOutput::new("Summary".to_string(), vec![]);

        let classifier = FakeClassifier { response: expected };
        let config = ClassifyConfig {
            prefilter_top_k: Some(1),
            ..Default::default()
        };
        let usecase = ClassifyUseCase::new(classifier, config);

        // Should succeed even with prefiltering
        let result = usecase
            .classify(&sample_post(), &sample_definitions())
            .await;
        assert!(result.is_ok());
    }
}
