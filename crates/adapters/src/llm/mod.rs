//! LLM provider adapters

pub mod anthropic;
pub mod claude_code;
pub mod codex;
pub mod gemini;
pub mod local_command;
pub mod ollama;
pub mod openai;
pub mod openai_compat;
pub mod opencode;
pub mod stub;

pub use anthropic::AnthropicClassifier;
pub use claude_code::ClaudeCodeClassifier;
pub use codex::CodexClassifier;
pub use gemini::GeminiClassifier;
pub use local_command::LocalCommandClassifier;
pub use ollama::OllamaClassifier;
pub use openai::OpenAiClassifier;
pub use openai_compat::OpenAiCompatClassifier;
pub use opencode::OpenCodeClassifier;
pub use stub::StubClassifier;

use serde::{Deserialize, Serialize};

/// Common LLM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Model name/ID
    pub model: String,
    /// Temperature (0.0-1.0)
    pub temperature: f64,
    /// Maximum output tokens
    pub max_output_tokens: u32,
    /// Request timeout in seconds
    pub timeout_secs: u64,
    /// Number of retries on failure
    pub retries: u32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            model: "gpt-4o-mini".to_string(),
            temperature: 0.2,
            max_output_tokens: 600,
            timeout_secs: 45,
            retries: 2,
        }
    }
}

/// Build the classification prompt
pub fn build_classification_prompt(
    post_text: &str,
    author: &str,
    definitions: &[news_tagger_domain::TagDefinition],
    policy_text: Option<&str>,
) -> String {
    let mut prompt = String::new();

    prompt.push_str("You are a narrative analysis system. Classify the following post against the provided tag definitions.\n\n");

    prompt.push_str("## Post to Analyze\n");
    prompt.push_str(&format!("Author: {}\n", author));
    prompt.push_str(&format!("Content: {}\n\n", post_text));

    prompt.push_str("## Tag Definitions\n");
    for def in definitions {
        prompt.push_str(&format!("### {} (ID: {})\n", def.title, def.id));
        if let Some(short) = &def.short {
            prompt.push_str(&format!("Summary: {}\n", short));
        }
        prompt.push_str(&format!("{}\n\n", def.content));
    }

    if let Some(policy) = policy_text {
        prompt.push_str("## Policy\n");
        prompt.push_str(policy);
        prompt.push_str("\n\n");
    }

    prompt.push_str(
        r#"## Output Format
Respond with ONLY a JSON object matching this exact schema:
{
  "version": "1",
  "summary": "1-2 sentence neutral summary of the post content",
  "tags": [
    {
      "id": "tag_id",
      "confidence": 0.0 to 1.0,
      "rationale": "Why this tag applies",
      "evidence": ["direct quote 1", "direct quote 2"]
    }
  ]
}

Rules:
- Only include tags with confidence >= 0.5
- Evidence must be direct quotes from the post
- If no tags apply, return empty tags array
- Be objective and neutral
"#,
    );

    prompt
}

/// Parse classification response JSON
pub fn parse_classification_response(
    response: &str,
) -> Result<news_tagger_domain::ClassifyOutput, String> {
    // Try to extract JSON from the response (in case of markdown code blocks)
    let json_str = extract_json(response);

    serde_json::from_str(json_str).map_err(|e| format!("Failed to parse JSON: {}", e))
}

/// Extract JSON from response (handles markdown code blocks)
fn extract_json(response: &str) -> &str {
    let trimmed = response.trim();

    // Check for ```json ... ``` blocks
    if let Some(start) = trimmed.find("```json") {
        if let Some(end) = trimmed[start + 7..].find("```") {
            return trimmed[start + 7..start + 7 + end].trim();
        }
    }

    // Check for ``` ... ``` blocks
    if let Some(start) = trimmed.find("```") {
        if let Some(end) = trimmed[start + 3..].find("```") {
            let content = trimmed[start + 3..start + 3 + end].trim();
            // Skip language identifier if present
            if let Some(newline) = content.find('\n') {
                let first_line = &content[..newline];
                if !first_line.starts_with('{') {
                    return content[newline + 1..].trim();
                }
            }
            return content;
        }
    }

    // Assume raw JSON
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json_raw() {
        let input = r#"{"version": "1", "tags": []}"#;
        assert_eq!(extract_json(input), input);
    }

    #[test]
    fn test_extract_json_code_block() {
        let input = r#"```json
{"version": "1", "tags": []}
```"#;
        assert_eq!(extract_json(input), r#"{"version": "1", "tags": []}"#);
    }

    #[test]
    fn test_parse_valid_response() {
        let json = r#"{
            "version": "1",
            "summary": "Test summary",
            "tags": [
                {
                    "id": "test_tag",
                    "confidence": 0.85,
                    "rationale": "Test rationale",
                    "evidence": ["quote 1"]
                }
            ]
        }"#;

        let result = parse_classification_response(json).unwrap();
        assert_eq!(result.version, "1");
        assert_eq!(result.tags.len(), 1);
        assert_eq!(result.tags[0].id, "test_tag");
    }
}
