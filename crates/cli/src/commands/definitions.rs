//! Definitions command - list and validate tag definitions

use anyhow::{Context, Result};
use news_tagger_adapters::definitions::FilesystemDefinitionsRepo;
use news_tagger_domain::DefinitionsRepo;
use std::path::PathBuf;

use crate::args::{DefinitionsArgs, DefinitionsCommands};
use crate::config::AppConfig;

pub async fn execute(args: DefinitionsArgs, config_path: Option<PathBuf>) -> Result<()> {
    match args.command {
        DefinitionsCommands::List {
            definitions_dir,
            json,
        } => list_definitions(definitions_dir, json, config_path).await,
        DefinitionsCommands::Validate { definitions_dir } => {
            validate_definitions(definitions_dir, config_path).await
        }
    }
}

async fn list_definitions(
    definitions_dir: Option<PathBuf>,
    json: bool,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref()).unwrap_or_default();
    let dir = definitions_dir
        .as_ref()
        .unwrap_or(&config.general.definitions_dir);

    let repo = FilesystemDefinitionsRepo::new(dir)
        .context("Failed to initialize definitions repository")?;

    let definitions = repo.load().await.context("Failed to load definitions")?;

    if json {
        let output = serde_json::json!({
            "count": definitions.len(),
            "definitions": definitions.iter().map(|d| serde_json::json!({
                "id": d.id,
                "title": d.title,
                "aliases": d.aliases,
                "short": d.short,
                "file_path": d.file_path,
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Tag Definitions ({} found)", definitions.len());
        println!("========================");
        println!();

        for def in &definitions {
            println!("ID: {}", def.id);
            println!("  Title: {}", def.title);
            if !def.aliases.is_empty() {
                println!("  Aliases: {}", def.aliases.join(", "));
            }
            if let Some(ref short) = def.short {
                println!("  Short: {}", short);
            }
            println!("  File: {}", def.file_path);
            println!();
        }
    }

    Ok(())
}

async fn validate_definitions(
    definitions_dir: Option<PathBuf>,
    config_path: Option<PathBuf>,
) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref()).unwrap_or_default();
    let dir = definitions_dir
        .as_ref()
        .unwrap_or(&config.general.definitions_dir);

    println!("Validating definitions in: {}", dir.display());

    let repo = FilesystemDefinitionsRepo::new(dir)
        .context("Failed to initialize definitions repository")?;

    match repo.validate().await {
        Ok(()) => {
            // Also load to get count
            let definitions = repo.load().await?;
            println!("✓ Validation passed ({} definitions)", definitions.len());
            Ok(())
        }
        Err(e) => {
            eprintln!("✗ Validation failed: {}", e);
            std::process::exit(1);
        }
    }
}
