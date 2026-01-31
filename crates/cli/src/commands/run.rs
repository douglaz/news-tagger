//! Run command - watch, classify, and publish loop

use anyhow::{Context, Result, bail};
use news_tagger_adapters::{
    definitions::FilesystemDefinitionsRepo,
    nostr::NostrPublisher,
    outbox::{OutboxPublisher, OutboxWriter},
    state::SqliteStateStore,
    x::{XPostSource, XPublisher},
};
use news_tagger_domain::{
    Classifier, ProcessResult, Publisher, SystemClock, XPublishMode,
    usecases::{ClassifyConfig, RenderConfig, RunLoop, RunLoopConfig},
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

use crate::args::RunArgs;
use crate::commands::classify::{build_classifier, load_api_key};
use crate::config::AppConfig;

pub async fn execute(args: RunArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref())?;

    let require_approval = args.require_approval;
    let outbox_path = if require_approval {
        Some(args.outbox.clone().unwrap_or_else(default_outbox_path))
    } else {
        None
    };

    if args.outbox.is_some() && !require_approval {
        tracing::warn!("--outbox is ignored without --require-approval");
    }

    let mut dry_run = args.dry_run || config.general.dry_run;
    if require_approval && dry_run {
        tracing::info!("--require-approval overrides dry-run");
        dry_run = false;
    }

    tracing::info!(
        dry_run = dry_run,
        once = args.once,
        require_approval = require_approval,
        outbox = ?outbox_path,
        accounts = ?config.watch.accounts,
        "Starting news-tagger run"
    );

    // Build dependencies
    let definitions_repo = Arc::new(
        FilesystemDefinitionsRepo::new(&config.general.definitions_dir)
            .context("Failed to initialize definitions repository")?,
    );

    let state_store = Arc::new(
        SqliteStateStore::new(&config.general.state_db_path)
            .await
            .context("Failed to initialize SQLite state store")?,
    );

    let post_source = Arc::new(build_post_source(&config)?);
    let classifier: Arc<dyn Classifier> = Arc::from(build_classifier(&config)?);

    let x_mode = parse_x_publish_mode(&config.x.write.mode)?;
    let (x_publisher, nostr_publisher): (Arc<dyn Publisher>, Arc<dyn Publisher>) =
        if require_approval {
            let outbox_path = outbox_path.expect("outbox path set when require_approval");
            let writer = OutboxWriter::new(outbox_path.clone())
                .await
                .context("Failed to initialize outbox writer")?;

            tracing::info!(
                outbox = %outbox_path.display(),
                "Writing approvals to outbox"
            );

            if !config.x.write.enabled && !config.nostr.enabled {
                tracing::warn!("Require approval enabled but no publishers are configured");
            }

            let x_publisher: Arc<dyn Publisher> = if config.x.write.enabled {
                Arc::new(OutboxPublisher::new(writer.clone(), "x"))
            } else {
                Arc::new(XPublisher::disabled())
            };

            let nostr_publisher: Arc<dyn Publisher> = if config.nostr.enabled {
                Arc::new(OutboxPublisher::new(writer, "nostr"))
            } else {
                Arc::new(NostrPublisher::disabled())
            };

            (x_publisher, nostr_publisher)
        } else {
            let x_publisher: Arc<dyn Publisher> =
                Arc::new(build_x_publisher(&config, dry_run, x_mode)?);
            let nostr_publisher: Arc<dyn Publisher> =
                Arc::new(build_nostr_publisher(&config, dry_run)?);
            (x_publisher, nostr_publisher)
        };

    let clock = Arc::new(SystemClock);

    // Build run loop configuration
    let loop_config = RunLoopConfig {
        accounts: config.watch.accounts.clone(),
        include_replies: config.watch.include_replies,
        include_reposts: config.watch.include_reposts,
        ignore_patterns: config.watch.ignore_patterns.clone(),
        dry_run,
        max_concurrent: config.general.max_concurrent,
        rate_limit_per_minute: rate_limit_from_config(config.general.rate_limit_per_minute),
        rate_limit_per_hour: rate_limit_from_config(config.general.rate_limit_per_hour),
        classify_config: classify_config_from_config(&config),
        render_config: RenderConfig {
            x_max_chars: config.x.write.max_chars,
            x_publish_mode: x_mode,
            ..Default::default()
        },
    };

    // Create run loop
    let run_loop = RunLoop::new(
        post_source,
        definitions_repo,
        classifier,
        x_publisher,
        nostr_publisher,
        state_store,
        clock,
        loop_config,
    );

    // Execute
    if args.once {
        tracing::info!("Running single poll cycle");
        let results = run_loop.poll_once().await?;
        tracing::info!(processed = results.len(), "Poll cycle complete");

        for (post_id, result) in results {
            match result {
                ProcessResult::Published {
                    classification,
                    x_post_id,
                    nostr_event_id,
                } => {
                    tracing::info!(
                        post_id = %post_id,
                        tags = ?classification.tags.iter().map(|t| &t.id).collect::<Vec<_>>(),
                        x_post_id = ?x_post_id,
                        nostr_event_id = ?nostr_event_id,
                        "Published"
                    );
                }
                ProcessResult::Skipped { reason } => {
                    tracing::debug!(post_id = %post_id, reason = %reason, "Skipped");
                }
                ProcessResult::Failed { error } => {
                    tracing::error!(post_id = %post_id, error = %error, "Failed");
                }
            }
        }
    } else {
        // Continuous polling loop
        let poll_interval = Duration::from_secs(config.watch.poll_interval_secs);
        let mut ticker = interval(poll_interval);

        // Set up graceful shutdown
        let shutdown = async {
            tokio::signal::ctrl_c()
                .await
                .expect("Failed to install Ctrl+C handler");
            tracing::info!("Shutdown signal received");
        };

        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match run_loop.poll_once().await {
                        Ok(results) => {
                            if !results.is_empty() {
                                tracing::info!(processed = results.len(), "Poll cycle complete");
                            }
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Poll cycle failed");
                        }
                    }
                }
                _ = &mut shutdown => {
                    tracing::info!("Shutting down gracefully");
                    break;
                }
            }
        }
    }

    tracing::info!("news-tagger run completed");
    Ok(())
}

fn build_post_source(config: &AppConfig) -> Result<XPostSource> {
    let bearer_token = load_api_key(&config.x.read.bearer_token_env, "x_read")?;
    Ok(XPostSource::new(bearer_token))
}

fn build_x_publisher(config: &AppConfig, dry_run: bool, mode: XPublishMode) -> Result<XPublisher> {
    if dry_run || !config.x.write.enabled {
        return Ok(XPublisher::disabled());
    }

    let user_token = load_api_key(&config.x.write.oauth2_user_token_env, "x_write")?;
    Ok(XPublisher::new(user_token, mode, config.x.write.max_chars))
}

fn build_nostr_publisher(config: &AppConfig, dry_run: bool) -> Result<NostrPublisher> {
    if dry_run || !config.nostr.enabled {
        return Ok(NostrPublisher::disabled());
    }

    if config.nostr.relays.is_empty() {
        bail!("Nostr enabled but no relays configured");
    }

    let secret_key = load_api_key(&config.nostr.secret_key_env, "nostr")?;
    Ok(NostrPublisher::new(secret_key, config.nostr.relays.clone()))
}

fn parse_x_publish_mode(mode: &str) -> Result<XPublishMode> {
    match mode.trim() {
        "reply" => Ok(XPublishMode::Reply),
        "quote" => Ok(XPublishMode::Quote),
        "new_post" => Ok(XPublishMode::NewPost),
        other => bail!("Invalid X publish mode: {}", other),
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

fn rate_limit_from_config(value: u32) -> Option<u32> {
    if value == 0 { None } else { Some(value) }
}

fn default_outbox_path() -> PathBuf {
    PathBuf::from("./outbox.jsonl")
}
