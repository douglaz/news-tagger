//! Rendering use case - transforms classification output into platform-specific content

use crate::model::{ClassifyOutput, RenderedPost, SourcePost, XPublishMode};

/// Configuration for the renderer
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Maximum characters for X posts
    pub x_max_chars: usize,
    /// X publishing mode
    pub x_publish_mode: XPublishMode,
    /// Whether to include confidence scores
    pub include_confidence: bool,
    /// Whether to include rationale
    pub include_rationale: bool,
    /// Minimum confidence to include a tag
    pub min_confidence: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            x_max_chars: 280,
            x_publish_mode: XPublishMode::Reply,
            include_confidence: true,
            include_rationale: true,
            min_confidence: 0.5,
        }
    }
}

/// Renderer for transforming classification output
pub struct Renderer {
    config: RenderConfig,
}

impl Renderer {
    pub fn new(config: RenderConfig) -> Self {
        Self { config }
    }

    /// Render classification output for X platform
    pub fn render_for_x(&self, post: &SourcePost, classification: &ClassifyOutput) -> RenderedPost {
        let tags_line = self.format_tags_line(classification);
        let rationale_line = self.format_rationale_line(classification);

        // Build content based on publish mode
        let content = match self.config.x_publish_mode {
            XPublishMode::Reply => {
                // Reply doesn't need the URL since it's threaded
                self.truncate_for_x(&format!("{}\n{}", tags_line, rationale_line))
            }
            XPublishMode::Quote => {
                // Quote also doesn't need URL (embedded)
                self.truncate_for_x(&format!("{}\n{}", tags_line, rationale_line))
            }
            XPublishMode::NewPost => {
                // Standalone post needs the URL
                let url_len = post.url.len() + 1; // +1 for newline
                let available = self.config.x_max_chars.saturating_sub(url_len);
                let main_content = self
                    .truncate_to_length(&format!("{}\n{}", tags_line, rationale_line), available);
                format!("{}\n{}", main_content, post.url)
            }
        };

        RenderedPost {
            text: content,
            source_post_id: post.id.clone(),
            source_post_url: post.url.clone(),
        }
    }

    /// Render classification output for Nostr
    pub fn render_for_nostr(
        &self,
        post: &SourcePost,
        classification: &ClassifyOutput,
    ) -> RenderedPost {
        let tags_line = self.format_tags_line(classification);
        let rationale = self.format_full_rationale(classification);

        let content = format!(
            "Narrative analysis of {}\n\n{}\n\n{}\n\nOriginal: {}",
            post.author, tags_line, rationale, post.url
        );

        RenderedPost {
            text: content,
            source_post_id: post.id.clone(),
            source_post_url: post.url.clone(),
        }
    }

    /// Format the tags line (e.g., "Tags: tag1 (0.82), tag2 (0.61)")
    fn format_tags_line(&self, classification: &ClassifyOutput) -> String {
        let filtered_tags: Vec<_> = classification
            .tags
            .iter()
            .filter(|t| t.confidence >= self.config.min_confidence)
            .collect();

        if filtered_tags.is_empty() {
            return "Tags: (none detected)".to_string();
        }

        let tag_strs: Vec<String> = filtered_tags
            .iter()
            .map(|t| {
                if self.config.include_confidence {
                    format!("{} ({:.2})", t.id, t.confidence)
                } else {
                    t.id.clone()
                }
            })
            .collect();

        format!("Tags: {}", tag_strs.join(", "))
    }

    /// Format a short rationale line for X
    fn format_rationale_line(&self, classification: &ClassifyOutput) -> String {
        if !self.config.include_rationale {
            return String::new();
        }

        // Use the first tag's rationale, truncated
        classification
            .tags
            .iter()
            .filter(|t| t.confidence >= self.config.min_confidence)
            .next()
            .map(|t| {
                // Truncate rationale to fit
                if t.rationale.len() > 100 {
                    format!("{}...", &t.rationale[..97])
                } else {
                    t.rationale.clone()
                }
            })
            .unwrap_or_default()
    }

    /// Format full rationale for Nostr (no length limit)
    fn format_full_rationale(&self, classification: &ClassifyOutput) -> String {
        let filtered: Vec<_> = classification
            .tags
            .iter()
            .filter(|t| t.confidence >= self.config.min_confidence)
            .collect();

        if filtered.is_empty() {
            return "No significant narrative patterns detected.".to_string();
        }

        filtered
            .iter()
            .map(|t| format!("• {}: {}", t.id, t.rationale))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Truncate content to fit X character limit
    fn truncate_for_x(&self, content: &str) -> String {
        self.truncate_to_length(content, self.config.x_max_chars)
    }

    /// Truncate to a specific length, preserving word boundaries
    fn truncate_to_length(&self, content: &str, max_len: usize) -> String {
        if content.len() <= max_len {
            return content.to_string();
        }

        // Find a good break point
        let truncate_at = max_len.saturating_sub(3); // Leave room for "..."
        let break_point = content[..truncate_at]
            .rfind(|c: char| c.is_whitespace() || c == '\n')
            .unwrap_or(truncate_at);

        format!("{}...", &content[..break_point])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TagMatch;
    use time::OffsetDateTime;

    fn sample_post() -> SourcePost {
        SourcePost {
            id: "123".to_string(),
            text: "Test post content".to_string(),
            author: "testuser".to_string(),
            url: "https://x.com/testuser/status/123".to_string(),
            created_at: OffsetDateTime::now_utc(),
            is_repost: false,
            is_reply: false,
            reply_to_id: None,
        }
    }

    fn sample_classification() -> ClassifyOutput {
        ClassifyOutput::new(
            "Summary of the post".to_string(),
            vec![
                TagMatch {
                    id: "tag_one".to_string(),
                    confidence: 0.85,
                    rationale: "First rationale explaining the match".to_string(),
                    evidence: vec!["evidence 1".to_string()],
                },
                TagMatch {
                    id: "tag_two".to_string(),
                    confidence: 0.62,
                    rationale: "Second rationale".to_string(),
                    evidence: vec!["evidence 2".to_string()],
                },
            ],
        )
    }

    #[test]
    fn test_render_for_x_reply_mode() {
        let renderer = Renderer::new(RenderConfig {
            x_publish_mode: XPublishMode::Reply,
            ..Default::default()
        });

        let result = renderer.render_for_x(&sample_post(), &sample_classification());

        assert!(result.text.contains("Tags:"));
        assert!(result.text.contains("tag_one"));
        assert!(result.text.contains("0.85"));
        assert!(!result.text.contains("https://")); // Reply mode doesn't include URL
    }

    #[test]
    fn test_render_for_x_new_post_mode() {
        let renderer = Renderer::new(RenderConfig {
            x_publish_mode: XPublishMode::NewPost,
            ..Default::default()
        });

        let result = renderer.render_for_x(&sample_post(), &sample_classification());

        assert!(result.text.contains("https://x.com/testuser/status/123"));
    }

    #[test]
    fn test_render_respects_max_chars() {
        let renderer = Renderer::new(RenderConfig {
            x_max_chars: 280,
            ..Default::default()
        });

        let result = renderer.render_for_x(&sample_post(), &sample_classification());

        assert!(
            result.text.len() <= 280,
            "Output too long: {}",
            result.text.len()
        );
    }

    #[test]
    fn test_render_filters_low_confidence() {
        let low_confidence = ClassifyOutput::new(
            "Summary".to_string(),
            vec![TagMatch {
                id: "low_tag".to_string(),
                confidence: 0.3,
                rationale: "Low confidence match".to_string(),
                evidence: vec![],
            }],
        );

        let renderer = Renderer::new(RenderConfig {
            min_confidence: 0.5,
            ..Default::default()
        });

        let result = renderer.render_for_x(&sample_post(), &low_confidence);

        assert!(result.text.contains("(none detected)"));
    }

    #[test]
    fn test_render_for_nostr() {
        let renderer = Renderer::new(RenderConfig::default());

        let result = renderer.render_for_nostr(&sample_post(), &sample_classification());

        assert!(result.text.contains("Narrative analysis"));
        assert!(result.text.contains("tag_one"));
        assert!(result.text.contains("Original:"));
        // Nostr has no strict length limit
    }
}
