//! Doctor command - validate configuration and show status

use anyhow::Result;
use news_tagger_adapters::definitions::FilesystemDefinitionsRepo;
use news_tagger_domain::DefinitionsRepo;
use serde::Serialize;
use std::path::PathBuf;

use crate::args::DoctorArgs;
use crate::config::AppConfig;

#[derive(Debug, Serialize)]
struct DoctorReport {
    config: CheckResult,
    definitions: CheckResult,
    llm: CheckResult,
    x_read: CheckResult,
    x_write: CheckResult,
    nostr: CheckResult,
    overall: String,
}

#[derive(Debug, Serialize)]
struct CheckResult {
    status: String,
    message: String,
    details: Option<serde_json::Value>,
}

impl CheckResult {
    fn ok(message: impl Into<String>) -> Self {
        Self {
            status: "ok".to_string(),
            message: message.into(),
            details: None,
        }
    }

    fn warn(message: impl Into<String>) -> Self {
        Self {
            status: "warn".to_string(),
            message: message.into(),
            details: None,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            message: message.into(),
            details: None,
        }
    }

    fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    fn is_ok(&self) -> bool {
        self.status == "ok"
    }

    fn is_error(&self) -> bool {
        self.status == "error"
    }
}

pub async fn execute(args: DoctorArgs, config_path: Option<PathBuf>) -> Result<()> {
    let mut report = DoctorReport {
        config: CheckResult::error("Not checked"),
        definitions: CheckResult::error("Not checked"),
        llm: CheckResult::error("Not checked"),
        x_read: CheckResult::error("Not checked"),
        x_write: CheckResult::error("Not checked"),
        nostr: CheckResult::error("Not checked"),
        overall: "error".to_string(),
    };

    // Check config
    let config = match AppConfig::load(config_path.as_deref()) {
        Ok(c) => {
            report.config = CheckResult::ok("Configuration loaded successfully");
            Some(c)
        }
        Err(e) => {
            report.config = CheckResult::error(format!("Failed to load config: {}", e));
            None
        }
    };

    if let Some(ref config) = config {
        // Check definitions
        report.definitions = check_definitions(&config.general.definitions_dir).await;

        // Check LLM
        report.llm = check_llm(config);

        // Check X read
        report.x_read = check_x_read(config);

        // Check X write
        report.x_write = check_x_write(config);

        // Check Nostr
        report.nostr = check_nostr(config);
    }

    // Determine overall status
    let checks = [
        &report.config,
        &report.definitions,
        &report.llm,
        &report.x_read,
    ];

    let has_error = checks.iter().any(|c| c.is_error());
    let all_ok = checks.iter().all(|c| c.is_ok());

    report.overall = if has_error {
        "error".to_string()
    } else if all_ok {
        "ok".to_string()
    } else {
        "warn".to_string()
    };

    // Output report
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report(&report);
    }

    if report.overall == "error" {
        std::process::exit(1);
    }

    Ok(())
}

async fn check_definitions(dir: &PathBuf) -> CheckResult {
    if !dir.exists() {
        return CheckResult::error(format!(
            "Definitions directory does not exist: {}",
            dir.display()
        ));
    }

    let repo = match FilesystemDefinitionsRepo::new(dir) {
        Ok(r) => r,
        Err(e) => {
            return CheckResult::error(format!("Failed to initialize definitions repo: {}", e));
        }
    };

    match repo.validate().await {
        Ok(()) => match repo.load().await {
            Ok(defs) => CheckResult::ok(format!("{} definitions loaded", defs.len())).with_details(
                serde_json::json!({
                    "count": defs.len(),
                    "ids": defs.iter().map(|d| &d.id).collect::<Vec<_>>()
                }),
            ),
            Err(e) => CheckResult::error(format!("Failed to load definitions: {}", e)),
        },
        Err(e) => CheckResult::error(format!("Validation failed: {}", e)),
    }
}

fn check_llm(config: &AppConfig) -> CheckResult {
    let provider = &config.llm.provider;
    let model = &config.llm.model;

    // Check if API key env var is set (without revealing the value)
    let api_key_env = match provider.as_str() {
        "openai" => &config.llm.openai.api_key_env,
        "anthropic" => &config.llm.anthropic.api_key_env,
        "gemini" => &config.llm.gemini.api_key_env,
        "ollama" => return CheckResult::ok(format!("Provider: ollama, Model: {}", model)),
        "claude_code" => {
            return check_cli_provider("claude_code", model, &config.llm.claude_code.command);
        }
        "codex" => return check_cli_provider("codex", model, &config.llm.codex.command),
        "opencode" => {
            return check_opencode_provider(model, &config.llm.opencode.base_url);
        }
        "stub" => return CheckResult::ok("Provider: stub (offline)".to_string()),
        "openai_compat" => &config.llm.openai_compat.api_key_env,
        other => return CheckResult::warn(format!("Unknown provider: {}", other)),
    };

    if api_key_env.is_empty() {
        return CheckResult::error(format!("No API key env var configured for {}", provider));
    }

    match std::env::var(api_key_env) {
        Ok(val) if !val.is_empty() => CheckResult::ok(format!(
            "Provider: {}, Model: {}, API key: {} (set)",
            provider, model, api_key_env
        )),
        _ => CheckResult::warn(format!(
            "Provider: {}, Model: {}, API key: {} (not set)",
            provider, model, api_key_env
        )),
    }
}

fn check_opencode_provider(model: &str, base_url: &str) -> CheckResult {
    if base_url.trim().is_empty() {
        return CheckResult::error("OpenCode base_url is empty");
    }

    CheckResult::ok(format!(
        "Provider: opencode, Model: {}, base_url: {}",
        model, base_url
    ))
}

fn check_cli_provider(provider: &str, model: &str, command: &str) -> CheckResult {
    if command.trim().is_empty() {
        return CheckResult::error(format!(
            "Provider: {}, Model: {}, command is empty",
            provider, model
        ));
    }

    if command_exists(command) {
        CheckResult::ok(format!(
            "Provider: {}, Model: {}, command: {}",
            provider, model, command
        ))
    } else {
        CheckResult::warn(format!(
            "Provider: {}, Model: {}, command not found on PATH: {}",
            provider, model, command
        ))
    }
}

fn command_exists(command: &str) -> bool {
    let path = std::path::Path::new(command);
    if path.components().count() > 1 {
        return path.is_file();
    }

    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };

    for dir in std::env::split_paths(&paths) {
        if dir.join(command).is_file() {
            return true;
        }
    }

    false
}

fn check_x_read(config: &AppConfig) -> CheckResult {
    let env_var = &config.x.read.bearer_token_env;

    if env_var.is_empty() {
        return CheckResult::error("No bearer token env var configured");
    }

    let accounts = &config.watch.accounts;
    if accounts.is_empty() {
        return CheckResult::warn("No accounts configured to watch");
    }

    match std::env::var(env_var) {
        Ok(val) if !val.is_empty() => CheckResult::ok(format!(
            "Bearer token: {} (set), Accounts: {}",
            env_var,
            accounts.join(", ")
        )),
        _ => CheckResult::warn(format!(
            "Bearer token: {} (not set), Accounts: {}",
            env_var,
            accounts.join(", ")
        )),
    }
}

fn check_x_write(config: &AppConfig) -> CheckResult {
    if !config.x.write.enabled {
        return CheckResult::ok("X write disabled");
    }

    let env_var = &config.x.write.oauth2_user_token_env;

    if env_var.is_empty() {
        return CheckResult::error("No user token env var configured");
    }

    match std::env::var(env_var) {
        Ok(val) if !val.is_empty() => CheckResult::ok(format!(
            "User token: {} (set), Mode: {}",
            env_var, config.x.write.mode
        )),
        _ => CheckResult::warn(format!(
            "User token: {} (not set), Mode: {}",
            env_var, config.x.write.mode
        )),
    }
}

fn check_nostr(config: &AppConfig) -> CheckResult {
    if !config.nostr.enabled {
        return CheckResult::ok("Nostr disabled");
    }

    let env_var = &config.nostr.secret_key_env;
    let relays = &config.nostr.relays;

    if env_var.is_empty() {
        return CheckResult::error("No secret key env var configured");
    }

    if relays.is_empty() {
        return CheckResult::warn("No relays configured");
    }

    match std::env::var(env_var) {
        Ok(val) if !val.is_empty() => CheckResult::ok(format!(
            "Secret key: {} (set), Relays: {}",
            env_var,
            relays.len()
        )),
        _ => CheckResult::warn(format!(
            "Secret key: {} (not set), Relays: {}",
            env_var,
            relays.len()
        )),
    }
}

fn print_report(report: &DoctorReport) {
    println!("news-tagger Doctor Report");
    println!("=========================");
    println!();

    print_check("Config", &report.config);
    print_check("Definitions", &report.definitions);
    print_check("LLM Provider", &report.llm);
    print_check("X Read", &report.x_read);
    print_check("X Write", &report.x_write);
    print_check("Nostr", &report.nostr);

    println!();
    let symbol = match report.overall.as_str() {
        "ok" => "✓",
        "warn" => "⚠",
        _ => "✗",
    };
    println!("{} Overall: {}", symbol, report.overall.to_uppercase());

    if report.overall == "ok" {
        println!();
        println!("Ready to run! Try: news-tagger run --dry-run --once");
    }
}

fn print_check(name: &str, result: &CheckResult) {
    let symbol = match result.status.as_str() {
        "ok" => "✓",
        "warn" => "⚠",
        _ => "✗",
    };
    println!("{} {}: {}", symbol, name, result.message);
}
