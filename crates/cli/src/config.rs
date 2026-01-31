//! Configuration loading and management

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub general: GeneralConfig,

    #[serde(default)]
    pub watch: WatchConfig,

    #[serde(default)]
    pub llm: LlmConfig,

    #[serde(default)]
    pub x: XConfig,

    #[serde(default)]
    pub nostr: NostrConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    #[serde(default = "default_definitions_dir")]
    pub definitions_dir: PathBuf,

    #[serde(default = "default_state_db_path")]
    pub state_db_path: PathBuf,

    #[serde(default = "default_log_level")]
    pub log_level: String,

    #[serde(default = "default_true")]
    pub dry_run: bool,

    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,

    #[serde(default = "default_rate_limit_per_minute")]
    pub rate_limit_per_minute: u32,

    #[serde(default = "default_rate_limit_per_hour")]
    pub rate_limit_per_hour: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,

    #[serde(default)]
    pub accounts: Vec<String>,

    #[serde(default)]
    pub include_replies: bool,

    #[serde(default)]
    pub include_reposts: bool,

    #[serde(default)]
    pub ignore_patterns: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_provider")]
    pub provider: String,

    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_temperature")]
    pub temperature: f64,

    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,

    #[serde(default = "default_llm_retries")]
    pub retries: u32,

    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,

    #[serde(default = "default_prefilter_top_k")]
    pub prefilter_top_k: usize,

    #[serde(default)]
    pub openai: OpenAiConfig,

    #[serde(default)]
    pub anthropic: AnthropicConfig,

    #[serde(default)]
    pub gemini: GeminiConfig,

    #[serde(default)]
    pub ollama: OllamaConfig,

    #[serde(default)]
    pub openai_compat: OpenAiCompatConfig,

    #[serde(default)]
    pub claude_code: ClaudeCodeConfig,

    #[serde(default)]
    pub codex: CodexConfig,

    #[serde(default)]
    pub opencode: OpenCodeConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAiConfig {
    #[serde(default = "default_openai_api_key_env")]
    pub api_key_env: String,

    #[serde(default = "default_openai_base_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnthropicConfig {
    #[serde(default = "default_anthropic_api_key_env")]
    pub api_key_env: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GeminiConfig {
    #[serde(default = "default_gemini_api_key_env")]
    pub api_key_env: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OllamaConfig {
    #[serde(default = "default_ollama_base_url")]
    pub base_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenAiCompatConfig {
    #[serde(default)]
    pub api_key_env: String,

    #[serde(default)]
    pub base_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenCodeConfig {
    #[serde(default = "default_opencode_base_url")]
    pub base_url: String,

    #[serde(default)]
    pub provider_id: String,

    #[serde(default)]
    pub model_id: String,

    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClaudeCodeConfig {
    #[serde(default = "default_claude_code_command")]
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CodexConfig {
    #[serde(default = "default_codex_command")]
    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XConfig {
    #[serde(default)]
    pub read: XReadConfig,

    #[serde(default)]
    pub write: XWriteConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct XReadConfig {
    #[serde(default = "default_x_bearer_token_env")]
    pub bearer_token_env: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XWriteConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_x_mode")]
    pub mode: String,

    #[serde(default = "default_x_user_token_env")]
    pub oauth2_user_token_env: String,

    #[serde(default = "default_x_max_chars")]
    pub max_chars: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostrConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_nostr_secret_key_env")]
    pub secret_key_env: String,

    #[serde(default)]
    pub relays: Vec<String>,
}

// Default value functions
fn default_definitions_dir() -> PathBuf {
    PathBuf::from("./definitions")
}

fn default_state_db_path() -> PathBuf {
    PathBuf::from("./state.sqlite")
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_true() -> bool {
    true
}

fn default_max_concurrent() -> usize {
    4
}

fn default_rate_limit_per_minute() -> u32 {
    0
}

fn default_rate_limit_per_hour() -> u32 {
    0
}

fn default_poll_interval() -> u64 {
    60
}

fn default_provider() -> String {
    "openai".to_string()
}

fn default_model() -> String {
    "gpt-4o-mini".to_string()
}

fn default_temperature() -> f64 {
    0.2
}

fn default_timeout() -> u64 {
    45
}

fn default_llm_retries() -> u32 {
    2
}

fn default_max_output_tokens() -> u32 {
    600
}

fn default_prefilter_top_k() -> usize {
    12
}

fn default_openai_api_key_env() -> String {
    "OPENAI_API_KEY".to_string()
}

fn default_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_anthropic_api_key_env() -> String {
    "ANTHROPIC_API_KEY".to_string()
}

fn default_gemini_api_key_env() -> String {
    "GEMINI_API_KEY".to_string()
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434".to_string()
}

fn default_x_bearer_token_env() -> String {
    "X_BEARER_TOKEN".to_string()
}

fn default_x_mode() -> String {
    "reply".to_string()
}

fn default_x_user_token_env() -> String {
    "X_USER_TOKEN".to_string()
}

fn default_x_max_chars() -> usize {
    280
}

fn default_nostr_secret_key_env() -> String {
    "NOSTR_NSEC".to_string()
}

fn default_claude_code_command() -> String {
    "claude".to_string()
}

fn default_codex_command() -> String {
    "codex".to_string()
}

fn default_opencode_base_url() -> String {
    "http://127.0.0.1:4096".to_string()
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            definitions_dir: default_definitions_dir(),
            state_db_path: default_state_db_path(),
            log_level: default_log_level(),
            dry_run: default_true(),
            max_concurrent: default_max_concurrent(),
            rate_limit_per_minute: default_rate_limit_per_minute(),
            rate_limit_per_hour: default_rate_limit_per_hour(),
        }
    }
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: default_poll_interval(),
            accounts: vec![],
            include_replies: false,
            include_reposts: false,
            ignore_patterns: vec![],
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            temperature: default_temperature(),
            timeout_secs: default_timeout(),
            retries: default_llm_retries(),
            max_output_tokens: default_max_output_tokens(),
            prefilter_top_k: default_prefilter_top_k(),
            openai: OpenAiConfig::default(),
            anthropic: AnthropicConfig::default(),
            gemini: GeminiConfig::default(),
            ollama: OllamaConfig::default(),
            openai_compat: OpenAiCompatConfig::default(),
            claude_code: ClaudeCodeConfig::default(),
            codex: CodexConfig::default(),
            opencode: OpenCodeConfig::default(),
        }
    }
}

impl Default for XWriteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: default_x_mode(),
            oauth2_user_token_env: default_x_user_token_env(),
            max_chars: default_x_max_chars(),
        }
    }
}

impl Default for NostrConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            secret_key_env: default_nostr_secret_key_env(),
            relays: vec![],
        }
    }
}

impl AppConfig {
    /// Load configuration from file and environment
    pub fn load(config_path: Option<&Path>) -> Result<Self> {
        let mut builder = config::Config::builder();

        // Try default config path if none specified
        let default_path = PathBuf::from("./config.toml");
        let path = config_path.unwrap_or(&default_path);

        if path.exists() {
            builder = builder.add_source(config::File::from(path));
        } else if config_path.is_some() {
            // User specified a path that doesn't exist
            anyhow::bail!("Config file not found: {}", path.display());
        }

        // Add environment variable overrides
        builder = builder.add_source(
            config::Environment::with_prefix("NEWS_TAGGER")
                .separator("__")
                .try_parsing(true),
        );

        let config = builder.build().context("Failed to build configuration")?;

        config
            .try_deserialize()
            .context("Failed to deserialize configuration")
    }

    /// Generate example configuration as TOML string
    pub fn example_toml() -> String {
        r#"# news-tagger configuration

[general]
definitions_dir = "./definitions"
state_db_path = "./state.sqlite"
log_level = "info"
dry_run = true
max_concurrent = 4
# 0 disables rate limiting
rate_limit_per_minute = 0
rate_limit_per_hour = 0

[watch]
poll_interval_secs = 60
accounts = ["example_account_1", "example_account_2"]
include_replies = false
include_reposts = false
# ignore_patterns = ["^RT @", "^AD:"]

[llm]
provider = "openai"  # openai, anthropic, gemini, ollama, openai_compat, claude_code, codex, opencode
model = "gpt-4o-mini"
temperature = 0.2
timeout_secs = 45
retries = 2
max_output_tokens = 600
prefilter_top_k = 12

[llm.openai]
api_key_env = "OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"

[llm.anthropic]
api_key_env = "ANTHROPIC_API_KEY"

[llm.gemini]
api_key_env = "GEMINI_API_KEY"

[llm.ollama]
base_url = "http://localhost:11434"

[llm.openai_compat]
api_key_env = "LLM_API_KEY"
base_url = "https://your-provider.com/v1"

[llm.claude_code]
command = "claude"
args = []
# timeout_secs = 45

[llm.codex]
command = "codex"
args = []
# timeout_secs = 45

[llm.opencode]
base_url = "http://127.0.0.1:4096"
# provider_id = "opencode"
# model_id = "big-pickle"
# timeout_secs = 45

[x.read]
bearer_token_env = "X_BEARER_TOKEN"

[x.write]
enabled = false
mode = "reply"  # reply, quote, new_post
oauth2_user_token_env = "X_USER_TOKEN"
max_chars = 280

[nostr]
enabled = false
secret_key_env = "NOSTR_NSEC"
relays = ["wss://relay.damus.io", "wss://nos.lol"]
"#
        .to_string()
    }
}
