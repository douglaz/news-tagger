//! Codex CLI adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};

use super::{LlmConfig, LocalCommandClassifier};

/// Codex classifier using local CLI.
pub struct CodexClassifier {
    inner: LocalCommandClassifier,
}

impl CodexClassifier {
    pub fn new(command: String, args: Vec<String>, config: LlmConfig) -> Self {
        let resolved = if command.trim().is_empty() {
            "codex".to_string()
        } else {
            command
        };
        Self {
            inner: LocalCommandClassifier::new(resolved, args, config),
        }
    }
}

#[async_trait]
impl Classifier for CodexClassifier {
    async fn classify(&self, input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
        self.inner.classify(input).await
    }
}
