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
        let resolved = resolve_command(command);
        Self {
            inner: LocalCommandClassifier::new(resolved, args, config),
        }
    }
}

fn resolve_command(command: String) -> String {
    if command.trim().is_empty() {
        "codex".to_string()
    } else {
        command
    }
}

#[async_trait]
impl Classifier for CodexClassifier {
    async fn classify(&self, input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
        self.inner.classify(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{ClassifyInput, SourcePost, TagDefinition};
    use time::OffsetDateTime;

    fn make_input() -> ClassifyInput {
        ClassifyInput {
            post: SourcePost {
                id: "1".to_string(),
                text: "test post".to_string(),
                author: "tester".to_string(),
                url: String::new(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            },
            definitions: vec![TagDefinition {
                id: "test_tag".to_string(),
                title: "Test Tag".to_string(),
                short: None,
                aliases: vec![],
                content: "Test definition".to_string(),
                file_path: "test.md".to_string(),
            }],
            max_output_chars: None,
            policy_text: None,
        }
    }

    #[test]
    fn test_default_command_fallback_for_empty_or_whitespace() {
        assert_eq!(resolve_command(String::new()), "codex");
        assert_eq!(resolve_command("   \n\t".to_string()), "codex");
    }

    #[test]
    fn test_custom_command_passthrough() {
        assert_eq!(
            resolve_command("my-codex-wrapper".to_string()),
            "my-codex-wrapper"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_successful_classification_via_mock_shell_command() {
        let script = r#"cat >/dev/null; printf '{"version":"1","summary":"ok","tags":[]}'"#;
        let classifier = CodexClassifier::new(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        );

        let output = classifier.classify(make_input()).await.unwrap();
        assert_eq!(output.version, "1");
        assert_eq!(output.summary, "ok");
        assert!(output.tags.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_timeout_propagation() {
        let script = r#"sleep 2; printf '{"version":"1","summary":"late","tags":[]}'"#;
        let classifier = CodexClassifier::new(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            LlmConfig {
                timeout_secs: 1,
                retries: 0,
                ..LlmConfig::default()
            },
        );

        let err = classifier.classify(make_input()).await.unwrap_err();
        assert!(matches!(err, ClassifyError::Timeout));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_non_zero_exit_propagation() {
        let script = r#"cat >/dev/null; echo "boom" >&2; exit 7"#;
        let classifier = CodexClassifier::new(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        );

        let err = classifier.classify(make_input()).await.unwrap_err();
        match err {
            ClassifyError::Api(message) => {
                assert!(message.contains("Command exited with"));
                assert!(message.contains("boom"));
            }
            other => panic!("Expected ClassifyError::Api, got {:?}", other),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_empty_stdout_propagation() {
        let script = r#"cat >/dev/null"#;
        let classifier = CodexClassifier::new(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        );

        let err = classifier.classify(make_input()).await.unwrap_err();
        match err {
            ClassifyError::InvalidFormat(message) => assert_eq!(message, "Empty response"),
            other => panic!("Expected ClassifyError::InvalidFormat, got {:?}", other),
        }
    }
}
