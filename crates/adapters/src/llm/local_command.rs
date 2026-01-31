//! Local command-based LLM adapter

use async_trait::async_trait;
use news_tagger_domain::{Classifier, ClassifyError, ClassifyInput, ClassifyOutput};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

use super::{LlmConfig, build_classification_prompt, parse_classification_response};

/// Classifier that shells out to a local CLI command.
pub struct LocalCommandClassifier {
    command: String,
    args: Vec<String>,
    config: LlmConfig,
}

impl LocalCommandClassifier {
    pub fn new(command: String, args: Vec<String>, config: LlmConfig) -> Self {
        Self {
            command,
            args,
            config,
        }
    }

    async fn run_command(&self, prompt: &str) -> Result<String, ClassifyError> {
        let ctx = CommandContext {
            prompt,
            model: &self.config.model,
            temperature: self.config.temperature,
            max_output_tokens: self.config.max_output_tokens,
        };
        let (expanded_args, used_prompt_arg) = expand_args(&self.args, &ctx);

        let mut command = Command::new(&self.command);
        command.args(&expanded_args);
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        if used_prompt_arg {
            command.stdin(Stdio::null());
        } else {
            command.stdin(Stdio::piped());
        }

        let mut child = command.spawn().map_err(|e| {
            ClassifyError::Api(format!("Failed to spawn command {}: {}", self.command, e))
        })?;

        if !used_prompt_arg {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(prompt.as_bytes()).await.map_err(|e| {
                    ClassifyError::Api(format!("Failed to write to stdin: {}", e))
                })?;
            }
        }

        let status = match tokio::time::timeout(
            Duration::from_secs(self.config.timeout_secs),
            child.wait(),
        )
        .await
        {
            Ok(result) => result.map_err(|e| ClassifyError::Api(e.to_string()))?,
            Err(_) => {
                let _ = child.kill().await;
                return Err(ClassifyError::Timeout);
            }
        };

        let mut stdout_bytes = Vec::new();
        if let Some(mut stdout) = child.stdout.take() {
            stdout
                .read_to_end(&mut stdout_bytes)
                .await
                .map_err(|e| ClassifyError::Api(e.to_string()))?;
        }

        let mut stderr_bytes = Vec::new();
        if let Some(mut stderr) = child.stderr.take() {
            stderr
                .read_to_end(&mut stderr_bytes)
                .await
                .map_err(|e| ClassifyError::Api(e.to_string()))?;
        }

        if !status.success() {
            let stderr = String::from_utf8_lossy(&stderr_bytes);
            return Err(ClassifyError::Api(format!(
                "Command exited with {}: {}",
                status, stderr
            )));
        }

        let stdout = String::from_utf8(stdout_bytes)
            .map_err(|e| ClassifyError::InvalidFormat(e.to_string()))?;
        if stdout.trim().is_empty() {
            return Err(ClassifyError::InvalidFormat("Empty response".to_string()));
        }

        Ok(stdout)
    }
}

#[async_trait]
impl Classifier for LocalCommandClassifier {
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

            match self.run_command(&prompt).await {
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

struct CommandContext<'a> {
    prompt: &'a str,
    model: &'a str,
    temperature: f64,
    max_output_tokens: u32,
}

fn expand_args(args: &[String], ctx: &CommandContext<'_>) -> (Vec<String>, bool) {
    let mut used_prompt_arg = false;
    let mut expanded = Vec::with_capacity(args.len());

    for arg in args {
        let mut value = arg.clone();
        if value.contains("{prompt}") {
            used_prompt_arg = true;
            value = value.replace("{prompt}", ctx.prompt);
        }
        if value.contains("{model}") {
            value = value.replace("{model}", ctx.model);
        }
        if value.contains("{temperature}") {
            value = value.replace("{temperature}", &ctx.temperature.to_string());
        }
        if value.contains("{max_output_tokens}") {
            value = value.replace("{max_output_tokens}", &ctx.max_output_tokens.to_string());
        }

        expanded.push(value);
    }

    (expanded, used_prompt_arg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{ClassifyInput, SourcePost, TagDefinition};
    use time::OffsetDateTime;

    #[test]
    fn test_expand_args_replaces_placeholders() {
        let args = vec![
            "--model".to_string(),
            "{model}".to_string(),
            "--temp={temperature}".to_string(),
            "--max={max_output_tokens}".to_string(),
            "{prompt}".to_string(),
        ];
        let ctx = CommandContext {
            prompt: "PROMPT",
            model: "test-model",
            temperature: 0.7,
            max_output_tokens: 256,
        };

        let (expanded, used_prompt) = expand_args(&args, &ctx);
        assert!(used_prompt);
        assert_eq!(
            expanded,
            vec![
                "--model",
                "test-model",
                "--temp=0.7",
                "--max=256",
                "PROMPT"
            ]
        );
    }

    #[tokio::test]
    async fn test_classify_with_local_command() {
        let script = r#"cat >/dev/null; printf '{"version":"1","summary":"ok","tags":[]}'"#;
        let classifier = LocalCommandClassifier::new(
            "sh".to_string(),
            vec!["-c".to_string(), script.to_string()],
            LlmConfig {
                retries: 0,
                ..LlmConfig::default()
            },
        );

        let input = ClassifyInput {
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
                content: "Test definition".to_string(),
                file_path: "test.md".to_string(),
                aliases: vec![],
            }],
            max_output_chars: None,
            policy_text: None,
        };

        let output = classifier.classify(input).await.unwrap();
        assert_eq!(output.version, "1");
        assert_eq!(output.summary, "ok");
        assert!(output.tags.is_empty());
    }
}
