//! Policy and safety constraints for outputs

use crate::model::ClassifyOutput;

/// Policy configuration
#[derive(Debug, Clone, Default)]
pub struct PolicyConfig {
    /// Maximum tags to include in output
    pub max_tags: Option<usize>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f64>,
    /// Maximum rationale length
    pub max_rationale_length: Option<usize>,
    /// Forbidden words in output (for safety)
    pub forbidden_patterns: Vec<String>,
}

/// Policy validator for classification outputs
pub struct PolicyValidator {
    config: PolicyConfig,
}

impl PolicyValidator {
    pub fn new(config: PolicyConfig) -> Self {
        Self { config }
    }

    /// Validate and optionally sanitize a classification output
    pub fn validate(&self, output: &ClassifyOutput) -> Result<ClassifyOutput, PolicyViolation> {
        let mut sanitized = output.clone();

        // Check for forbidden patterns
        for pattern in &self.config.forbidden_patterns {
            let pattern_lower = pattern.to_lowercase();
            if sanitized.summary.to_lowercase().contains(&pattern_lower) {
                return Err(PolicyViolation::ForbiddenPattern {
                    pattern: pattern.clone(),
                    context: "summary".to_string(),
                });
            }
            for tag in &sanitized.tags {
                if tag.rationale.to_lowercase().contains(&pattern_lower) {
                    return Err(PolicyViolation::ForbiddenPattern {
                        pattern: pattern.clone(),
                        context: format!("tag {} rationale", tag.id),
                    });
                }
            }
        }

        // Filter by confidence
        if let Some(min_conf) = self.config.min_confidence {
            sanitized.tags.retain(|t| t.confidence >= min_conf);
        }

        // Limit number of tags
        if let Some(max_tags) = self.config.max_tags {
            sanitized.tags.truncate(max_tags);
        }

        // Truncate rationales
        if let Some(max_len) = self.config.max_rationale_length {
            for tag in &mut sanitized.tags {
                if tag.rationale.len() > max_len {
                    tag.rationale = format!("{}...", &tag.rationale[..max_len - 3]);
                }
            }
        }

        Ok(sanitized)
    }

    /// Generate policy text for LLM prompt
    pub fn generate_policy_prompt(&self) -> String {
        let mut lines = vec![
            "Output Policy:".to_string(),
            "- Be objective and neutral in rationales".to_string(),
            "- Evidence must be direct quotes from the post".to_string(),
            "- Do not make claims about intent or motivation".to_string(),
        ];

        if let Some(max_tags) = self.config.max_tags {
            lines.push(format!("- Maximum {} tags per classification", max_tags));
        }

        if let Some(min_conf) = self.config.min_confidence {
            lines.push(format!(
                "- Only include tags with confidence >= {:.2}",
                min_conf
            ));
        }

        lines.join("\n")
    }
}

/// Policy violation errors
#[derive(Debug, thiserror::Error)]
pub enum PolicyViolation {
    #[error("Forbidden pattern '{pattern}' found in {context}")]
    ForbiddenPattern { pattern: String, context: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TagMatch;

    fn sample_output() -> ClassifyOutput {
        ClassifyOutput::new(
            "A post about climate policy".to_string(),
            vec![
                TagMatch {
                    id: "climate_fear".to_string(),
                    confidence: 0.9,
                    rationale: "Uses fear-based language".to_string(),
                    evidence: vec!["we're doomed".to_string()],
                },
                TagMatch {
                    id: "low_confidence".to_string(),
                    confidence: 0.3,
                    rationale: "Weak match".to_string(),
                    evidence: vec![],
                },
            ],
        )
    }

    #[test]
    fn test_policy_filters_low_confidence() {
        let validator = PolicyValidator::new(PolicyConfig {
            min_confidence: Some(0.5),
            ..Default::default()
        });

        let result = validator.validate(&sample_output()).unwrap();
        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "climate_fear");
    }

    #[test]
    fn test_policy_limits_tags() {
        let validator = PolicyValidator::new(PolicyConfig {
            max_tags: Some(1),
            ..Default::default()
        });

        let result = validator.validate(&sample_output()).unwrap();
        assert_eq!(result.tags.len(), 1);
    }

    #[test]
    fn test_policy_forbidden_patterns() {
        let validator = PolicyValidator::new(PolicyConfig {
            forbidden_patterns: vec!["climate".to_string()],
            ..Default::default()
        });

        let result = validator.validate(&sample_output());
        assert!(matches!(
            result,
            Err(PolicyViolation::ForbiddenPattern { .. })
        ));
    }

    #[test]
    fn test_policy_truncates_rationale() {
        let mut output = sample_output();
        output.tags[0].rationale = "A".repeat(200);

        let validator = PolicyValidator::new(PolicyConfig {
            max_rationale_length: Some(50),
            ..Default::default()
        });

        let result = validator.validate(&output).unwrap();
        assert!(result.tags[0].rationale.len() <= 50);
        assert!(result.tags[0].rationale.ends_with("..."));
    }
}
