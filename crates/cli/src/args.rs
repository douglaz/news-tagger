//! CLI argument definitions

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// news-tagger: CLI tool for classifying posts using LLM-powered narrative tagging
#[derive(Parser, Debug)]
#[command(name = "news-tagger")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Path to configuration file
    #[arg(short, long, global = true)]
    pub config: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, global = true)]
    pub log_level: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Watch accounts, classify posts, and publish results
    Run(RunArgs),

    /// One-shot classification of text
    Classify(ClassifyArgs),

    /// Manage tag definitions
    Definitions(DefinitionsArgs),

    /// Configuration management
    Config(ConfigArgs),

    /// Validate configuration and show status
    Doctor(DoctorArgs),
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Run in dry-run mode (no actual publishing)
    #[arg(long)]
    pub dry_run: bool,

    /// Process one poll cycle and exit
    #[arg(long)]
    pub once: bool,

    /// Write rendered posts to outbox file for review instead of publishing
    #[arg(long)]
    pub require_approval: bool,

    /// Path to outbox file (used with --require-approval)
    #[arg(long)]
    pub outbox: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct ClassifyArgs {
    /// Text to classify
    #[arg(long, conflicts_with = "file")]
    pub text: Option<String>,

    /// File containing text to classify (use - for stdin)
    #[arg(long, conflicts_with = "text")]
    pub file: Option<PathBuf>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,

    /// Override definitions directory
    #[arg(long)]
    pub definitions_dir: Option<PathBuf>,
}

#[derive(Args, Debug)]
pub struct DefinitionsArgs {
    #[command(subcommand)]
    pub command: DefinitionsCommands,
}

#[derive(Subcommand, Debug)]
pub enum DefinitionsCommands {
    /// List all loaded definitions
    List {
        /// Override definitions directory
        #[arg(long)]
        definitions_dir: Option<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Validate definitions
    Validate {
        /// Override definitions directory
        #[arg(long)]
        definitions_dir: Option<PathBuf>,
    },
}

#[derive(Args, Debug)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommands,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommands {
    /// Generate example configuration file
    Init {
        /// Path to write config file
        #[arg(long, default_value = "./config.toml")]
        path: PathBuf,

        /// Overwrite existing file
        #[arg(long)]
        force: bool,
    },
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// Check specific component (config, definitions, llm, x, nostr)
    #[arg(long)]
    pub check: Option<String>,

    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}
