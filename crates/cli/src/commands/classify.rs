//! Classify command - one-shot classification

use anyhow::{Context, Result, bail};
use news_tagger_adapters::{
    definitions::FilesystemDefinitionsRepo,
    llm::{
        AnthropicClassifier, ClaudeCodeClassifier, CodexClassifier, GeminiClassifier,
        LlmConfig as AdapterLlmConfig, OllamaClassifier, OpenAiClassifier, OpenAiCompatClassifier,
        OpenCodeClassifier, StubClassifier,
    },
};
use news_tagger_domain::usecases::{ClassifyConfig, ClassifyUseCase};
use news_tagger_domain::{Classifier, DefinitionsRepo, SourcePost};
use secrecy::SecretString;
use std::io::{self, Read};
use std::path::PathBuf;
use time::OffsetDateTime;

use crate::args::ClassifyArgs;
use crate::config::AppConfig;

pub async fn execute(args: ClassifyArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref()).unwrap_or_default();

    // Get text to classify
    let text = get_input_text(&args)?;

    if text.trim().is_empty() {
        anyhow::bail!("No text provided for classification");
    }

    // Load definitions
    let definitions_dir = args
        .definitions_dir
        .as_ref()
        .unwrap_or(&config.general.definitions_dir);

    let definitions_repo = FilesystemDefinitionsRepo::new(definitions_dir)
        .context("Failed to initialize definitions repository")?;

    let definitions = definitions_repo
        .load()
        .await
        .context("Failed to load definitions")?;

    if definitions.is_empty() {
        anyhow::bail!("No definitions found in {}", definitions_dir.display());
    }

    tracing::info!(
        definitions_count = definitions.len(),
        text_length = text.len(),
        "Classifying text"
    );

    // Create a synthetic source post
    let post = SourcePost {
        id: "cli-input".to_string(),
        text: text.clone(),
        author: "cli".to_string(),
        url: String::new(),
        created_at: OffsetDateTime::now_utc(),
        is_repost: false,
        is_reply: false,
        reply_to_id: None,
    };

    // Run classification (same prefilter behavior as main loop)
    let classifier = build_classifier(&config)?;
    let classify_config = classify_config_from_config(&config);
    let usecase = ClassifyUseCase::new(&*classifier, classify_config);
    let output = usecase
        .classify(&post, &definitions)
        .await
        .context("Classification failed")?;

    // Output results
    if args.json {
        let json = serde_json::to_string_pretty(&output).context("Failed to serialize output")?;
        println!("{}", json);
    } else {
        println!("Classification Results");
        println!("======================");
        println!();
        println!("Summary: {}", output.summary);
        println!();

        if output.tags.is_empty() {
            println!("No tags matched.");
        } else {
            println!("Tags:");
            for tag in &output.tags {
                println!("  - {} (confidence: {:.2})", tag.id, tag.confidence);
                println!("    Rationale: {}", tag.rationale);
                if !tag.evidence.is_empty() {
                    println!("    Evidence:");
                    for e in &tag.evidence {
                        println!("      - \"{}\"", e);
                    }
                }
                println!();
            }
        }
    }

    Ok(())
}

pub(crate) fn build_classifier(config: &AppConfig) -> Result<Box<dyn Classifier>> {
    let llm_config = adapter_llm_config(&config.llm);

    match config.llm.provider.as_str() {
        "openai" => {
            let api_key = load_api_key(&config.llm.openai.api_key_env, "openai")?;
            Ok(Box::new(OpenAiClassifier::with_base_url(
                api_key,
                config.llm.openai.base_url.clone(),
                llm_config.clone(),
            )))
        }
        "anthropic" => {
            let api_key = load_api_key(&config.llm.anthropic.api_key_env, "anthropic")?;
            Ok(Box::new(AnthropicClassifier::new(
                api_key,
                llm_config.clone(),
            )))
        }
        "gemini" => {
            let api_key = load_api_key(&config.llm.gemini.api_key_env, "gemini")?;
            Ok(Box::new(GeminiClassifier::new(api_key, llm_config.clone())))
        }
        "ollama" => {
            let base_url = config.llm.ollama.base_url.trim();
            if base_url.is_empty() {
                Ok(Box::new(OllamaClassifier::new(llm_config.clone())))
            } else {
                Ok(Box::new(OllamaClassifier::with_base_url(
                    base_url.to_string(),
                    llm_config.clone(),
                )))
            }
        }
        "openai_compat" => {
            let base_url = config.llm.openai_compat.base_url.trim();
            if base_url.is_empty() {
                bail!("OpenAI-compatible base_url is required");
            }
            let api_key = load_api_key(&config.llm.openai_compat.api_key_env, "openai_compat")?;
            Ok(Box::new(OpenAiCompatClassifier::new(
                api_key,
                base_url.to_string(),
                llm_config.clone(),
            )))
        }
        "opencode" => {
            let mut local_config = llm_config.clone();
            if let Some(timeout) = config.llm.opencode.timeout_secs {
                local_config.timeout_secs = timeout;
            }
            Ok(Box::new(
                OpenCodeClassifier::new(
                    config.llm.opencode.base_url.clone(),
                    non_empty(&config.llm.opencode.provider_id),
                    non_empty(&config.llm.opencode.model_id),
                    None,
                    local_config,
                )
                .context("Failed to configure OpenCode provider")?,
            ))
        }
        "claude_code" => {
            let mut local_config = llm_config.clone();
            if let Some(timeout) = config.llm.claude_code.timeout_secs {
                local_config.timeout_secs = timeout;
            }
            Ok(Box::new(ClaudeCodeClassifier::new(
                config.llm.claude_code.command.clone(),
                config.llm.claude_code.args.clone(),
                local_config,
            )))
        }
        "codex" => {
            let mut local_config = llm_config.clone();
            if let Some(timeout) = config.llm.codex.timeout_secs {
                local_config.timeout_secs = timeout;
            }
            Ok(Box::new(CodexClassifier::new(
                config.llm.codex.command.clone(),
                config.llm.codex.args.clone(),
                local_config,
            )))
        }
        "stub" => Ok(Box::new(StubClassifier::echo())),
        other => bail!("Unknown LLM provider: {}", other),
    }
}

fn adapter_llm_config(config: &crate::config::LlmConfig) -> AdapterLlmConfig {
    AdapterLlmConfig {
        model: config.model.clone(),
        temperature: config.temperature,
        max_output_tokens: config.max_output_tokens,
        timeout_secs: config.timeout_secs,
        retries: config.retries,
    }
}

fn classify_config_from_config(config: &AppConfig) -> ClassifyConfig {
    let prefilter_top_k = if config.llm.prefilter_top_k == 0 {
        None
    } else {
        Some(config.llm.prefilter_top_k)
    };

    ClassifyConfig {
        prefilter_top_k,
        policy_text: None,
        max_output_chars: Some(config.x.write.max_chars),
    }
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub(crate) fn load_api_key(env_var: &str, provider: &str) -> Result<SecretString> {
    if env_var.trim().is_empty() {
        bail!("No API key env var configured for provider {}", provider);
    }

    let key = std::env::var(env_var).with_context(|| {
        format!(
            "Missing API key env var {} for provider {}",
            env_var, provider
        )
    })?;

    if key.trim().is_empty() {
        bail!(
            "API key env var {} is empty for provider {}",
            env_var,
            provider
        );
    }

    Ok(SecretString::new(key.into()))
}

fn get_input_text(args: &ClassifyArgs) -> Result<String> {
    if let Some(ref text) = args.text {
        return Ok(text.clone());
    }

    if let Some(ref path) = args.file {
        if path.as_os_str() == "-" {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .context("Failed to read from stdin")?;
            return Ok(text);
        }

        return std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read file: {}", path.display()));
    }

    // Default to stdin if no input specified
    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .context("Failed to read from stdin")?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use news_tagger_domain::{ClassifyError, ClassifyInput, SourcePost, TagDefinition};
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
    fn test_build_classifier_selects_codex_provider_and_returns_ok() {
        let mut config = AppConfig::default();
        config.llm.provider = "codex".to_string();
        config.llm.codex.command = "codex-custom".to_string();
        config.llm.codex.args = vec!["exec".to_string(), "-".to_string()];

        let classifier = build_classifier(&config);
        assert!(classifier.is_ok());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_build_classifier_honors_codex_timeout_override() {
        let mut config = AppConfig::default();
        config.llm.provider = "codex".to_string();
        config.llm.timeout_secs = 30;
        config.llm.codex.timeout_secs = Some(1);
        config.llm.codex.command = "sh".to_string();
        config.llm.codex.args = vec![
            "-c".to_string(),
            "sleep 2; printf '{\"version\":\"1\",\"summary\":\"late\",\"tags\":[]}'".to_string(),
        ];

        let classifier = build_classifier(&config).unwrap();
        let err = classifier.classify(make_input()).await.unwrap_err();
        assert!(matches!(err, ClassifyError::Timeout));
    }
}
