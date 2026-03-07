//! Fetch command - collect posts from X and save as JSONL

use anyhow::{Context, Result};
use news_tagger_adapters::x::XPostSource;
use news_tagger_domain::{PostSource, SourcePost};
use std::path::PathBuf;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::args::FetchArgs;
use crate::commands::classify::load_api_key;
use crate::config::AppConfig;

pub async fn execute(args: FetchArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref())?;

    let accounts = args
        .accounts
        .unwrap_or_else(|| config.watch.accounts.clone());

    if accounts.is_empty() {
        anyhow::bail!("No accounts configured. Use --accounts or set watch.accounts in config.");
    }

    let bearer_token = load_api_key(&config.x.read.bearer_token_env, "x_read")?;
    let post_source = XPostSource::new(bearer_token);

    // Load existing posts to find the latest ID per account (for incremental fetches)
    let existing_max_ids = load_max_ids_per_account(&args.output);

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&args.output)
        .await
        .with_context(|| format!("Failed to open {}", args.output.display()))?;

    let mut total = 0usize;

    for account in &accounts {
        let account_lower = account.to_ascii_lowercase();
        let since_id = existing_max_ids.get(&account_lower).map(|s| s.as_str());

        tracing::info!(account = %account, since_id = ?since_id, "Fetching posts");

        match post_source.fetch_posts(account, since_id).await {
            Ok(posts) => {
                let mut fetched = 0usize;
                for post in &posts {
                    if !config.watch.include_replies && post.is_reply {
                        continue;
                    }
                    if !config.watch.include_reposts && post.is_repost {
                        continue;
                    }

                    let line = serde_json::to_string(post).context("Failed to serialize post")?;
                    file.write_all(line.as_bytes()).await?;
                    file.write_all(b"\n").await?;
                    fetched += 1;
                }
                // Write a cursor entry so skipped posts still advance the since_id
                if let Some(last) = posts.last() {
                    let cursor = SinceIdCursor {
                        _cursor: true,
                        account: account.clone(),
                        max_id: last.id.clone(),
                    };
                    let line =
                        serde_json::to_string(&cursor).context("Failed to serialize cursor")?;
                    file.write_all(line.as_bytes()).await?;
                    file.write_all(b"\n").await?;
                }
                file.flush().await?;
                tracing::info!(account = %account, fetched = fetched, "Saved posts");
                total += fetched;
            }
            Err(e) => {
                tracing::error!(account = %account, error = %e, "Failed to fetch");
            }
        }
    }

    tracing::info!(
        total = total,
        output = %args.output.display(),
        "Fetch complete"
    );

    Ok(())
}

/// Cursor entry written to JSONL to track the latest fetched post ID,
/// even when all posts in a batch were filtered out.
#[derive(serde::Serialize, serde::Deserialize)]
struct SinceIdCursor {
    _cursor: bool,
    account: String,
    max_id: String,
}

/// Scan existing JSONL file to find the max post ID per account (for incremental fetches).
/// Uses lowercase account keys for case-insensitive matching.
fn load_max_ids_per_account(path: &PathBuf) -> std::collections::HashMap<String, String> {
    let mut max_ids = std::collections::HashMap::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return max_ids,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Try cursor entries first
        if let Ok(cursor) = serde_json::from_str::<SinceIdCursor>(line) {
            if cursor._cursor {
                let key = cursor.account.to_ascii_lowercase();
                let entry = max_ids.entry(key).or_insert_with(String::new);
                if cursor.max_id.as_str() > entry.as_str() {
                    *entry = cursor.max_id;
                }
                continue;
            }
        }
        // Regular post entries
        if let Ok(post) = serde_json::from_str::<SourcePost>(line) {
            let key = post.author.to_ascii_lowercase();
            let entry = max_ids.entry(key).or_insert_with(String::new);
            if post.id.as_str() > entry.as_str() {
                *entry = post.id;
            }
        }
    }
    max_ids
}
