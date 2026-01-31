//! Run loop use case - orchestrates watching, classifying, and publishing

use std::sync::Arc;
use uuid::Uuid;

use crate::{
    model::{AccountState, ProcessResult, PublishedRecord, SourcePost, Taxonomy},
    ports::{Classifier, ClassifyError, Clock, DefinitionsRepo, PostSource, Publisher, StateStore},
    usecases::{
        classify::{ClassifyConfig, ClassifyUseCase},
        render::{RenderConfig, Renderer},
    },
};
use futures::future::BoxFuture;
use futures::stream::{FuturesUnordered, StreamExt};
use regex::Regex;
use tokio::sync::Mutex;
use tokio::time::{Duration, Instant, sleep};

/// Configuration for the run loop
#[derive(Debug, Clone)]
pub struct RunLoopConfig {
    /// Accounts to watch
    pub accounts: Vec<String>,
    /// Whether to include replies
    pub include_replies: bool,
    /// Whether to include reposts
    pub include_reposts: bool,
    /// Regex patterns for posts to ignore
    pub ignore_patterns: Vec<String>,
    /// Dry run mode (don't actually publish)
    pub dry_run: bool,
    /// Maximum concurrent post processing tasks
    pub max_concurrent: usize,
    /// Max posts processed per minute (None = unlimited)
    pub rate_limit_per_minute: Option<u32>,
    /// Max posts processed per hour (None = unlimited)
    pub rate_limit_per_hour: Option<u32>,
    /// Classification config
    pub classify_config: ClassifyConfig,
    /// Render config
    pub render_config: RenderConfig,
}

impl Default for RunLoopConfig {
    fn default() -> Self {
        Self {
            accounts: vec![],
            include_replies: false,
            include_reposts: false,
            ignore_patterns: vec![],
            dry_run: true,
            max_concurrent: 4,
            rate_limit_per_minute: None,
            rate_limit_per_hour: None,
            classify_config: ClassifyConfig::default(),
            render_config: RenderConfig::default(),
        }
    }
}

/// Run loop orchestrator
#[derive(Clone)]
pub struct RunLoop<S, D, C, X, N, St, Cl>
where
    S: PostSource + ?Sized,
    D: DefinitionsRepo + ?Sized,
    C: Classifier + ?Sized,
    X: Publisher + ?Sized,
    N: Publisher + ?Sized,
    St: StateStore + ?Sized,
    Cl: Clock + ?Sized,
{
    post_source: Arc<S>,
    definitions_repo: Arc<D>,
    classifier: Arc<C>,
    x_publisher: Arc<X>,
    nostr_publisher: Arc<N>,
    state_store: Arc<St>,
    clock: Arc<Cl>,
    config: RunLoopConfig,
    ignore_patterns: Vec<Regex>,
    rate_limiter: Arc<RateLimiter>,
}

impl<S, D, C, X, N, St, Cl> RunLoop<S, D, C, X, N, St, Cl>
where
    S: PostSource + ?Sized,
    D: DefinitionsRepo + ?Sized,
    C: Classifier + ?Sized,
    X: Publisher + ?Sized,
    N: Publisher + ?Sized,
    St: StateStore + ?Sized,
    Cl: Clock + ?Sized,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        post_source: Arc<S>,
        definitions_repo: Arc<D>,
        classifier: Arc<C>,
        x_publisher: Arc<X>,
        nostr_publisher: Arc<N>,
        state_store: Arc<St>,
        clock: Arc<Cl>,
        config: RunLoopConfig,
    ) -> Self {
        let ignore_patterns = compile_ignore_patterns(&config.ignore_patterns);
        let rate_limiter = Arc::new(RateLimiter::new(
            config.rate_limit_per_minute,
            config.rate_limit_per_hour,
        ));
        Self {
            post_source,
            definitions_repo,
            classifier,
            x_publisher,
            nostr_publisher,
            state_store,
            clock,
            config,
            ignore_patterns,
            rate_limiter,
        }
    }

    /// Run a single poll cycle for all accounts
    pub async fn poll_once(&self) -> Result<Vec<(String, ProcessResult)>, RunLoopError> {
        // Load definitions
        let definitions = self
            .definitions_repo
            .load()
            .await
            .map_err(|e| RunLoopError::Definitions(e.to_string()))?;

        let taxonomy = Arc::new(Taxonomy::new(definitions));

        tracing::info!(
            taxonomy_hash = %taxonomy.hash,
            definition_count = taxonomy.definitions.len(),
            "Loaded taxonomy"
        );

        let mut results = Vec::new();

        for account in &self.config.accounts {
            match self.poll_account(account, Arc::clone(&taxonomy)).await {
                Ok(account_results) => results.extend(account_results),
                Err(e) => {
                    tracing::error!(account = %account, error = %e, "Failed to poll account");
                    // Continue with other accounts
                }
            }
        }

        Ok(results)
    }

    /// Poll a single account
    async fn poll_account(
        &self,
        account: &str,
        taxonomy: Arc<Taxonomy>,
    ) -> Result<Vec<(String, ProcessResult)>, RunLoopError> {
        // Get last processed ID
        let account_state = self
            .state_store
            .get_account_state(account)
            .await
            .map_err(|e| RunLoopError::State(e.to_string()))?;

        let since_id = account_state.as_ref().and_then(|s| s.since_id.as_deref());

        tracing::info!(
            account = %account,
            since_id = ?since_id,
            "Fetching posts"
        );

        // Fetch new posts
        let posts = self
            .post_source
            .fetch_posts(account, since_id)
            .await
            .map_err(|e| RunLoopError::PostSource(e.to_string()))?;

        if posts.is_empty() {
            tracing::debug!(account = %account, "No new posts");
            return Ok(vec![]);
        }

        tracing::info!(account = %account, count = posts.len(), "Fetched posts");

        // Filter posts
        let filtered_posts = self.filter_posts(posts);

        if filtered_posts.is_empty() {
            return Ok(vec![]);
        }

        // Process each post with bounded concurrency and rate limiting
        let mut results = Vec::new();
        let mut last_id = since_id.map(String::from);
        let max_concurrent = self.config.max_concurrent.max(1);
        let mut tasks: FuturesUnordered<BoxFuture<'_, (String, ProcessResult)>> =
            FuturesUnordered::new();
        let mut posts_iter = filtered_posts.into_iter();

        while tasks.len() < max_concurrent {
            let Some(post) = posts_iter.next() else {
                break;
            };
            last_id = Some(post.id.clone());
            let rate_limiter = Arc::clone(&self.rate_limiter);
            let taxonomy = Arc::clone(&taxonomy);
            tasks.push(Box::pin(async move {
                rate_limiter.acquire().await;
                let result = self.process_post(&post, taxonomy.as_ref()).await;
                (post.id, result)
            }));
        }

        while let Some(result) = tasks.next().await {
            results.push(result);
            while tasks.len() < max_concurrent {
                let Some(post) = posts_iter.next() else {
                    break;
                };
                last_id = Some(post.id.clone());
                let rate_limiter = Arc::clone(&self.rate_limiter);
                let taxonomy = Arc::clone(&taxonomy);
                tasks.push(Box::pin(async move {
                    rate_limiter.acquire().await;
                    let result = self.process_post(&post, taxonomy.as_ref()).await;
                    (post.id, result)
                }));
            }
        }

        // Update since_id
        if let Some(last_id) = last_id {
            let new_state = AccountState {
                account: account.to_string(),
                since_id: Some(last_id),
                updated_at: self.clock.now(),
            };
            self.state_store
                .set_account_state(&new_state)
                .await
                .map_err(|e| RunLoopError::State(e.to_string()))?;
        }

        Ok(results)
    }

    /// Filter posts based on config
    fn filter_posts(&self, posts: Vec<SourcePost>) -> Vec<SourcePost> {
        posts
            .into_iter()
            .filter(|p| {
                if !self.config.include_replies && p.is_reply {
                    return false;
                }
                if !self.config.include_reposts && p.is_repost {
                    return false;
                }
                if self
                    .ignore_patterns
                    .iter()
                    .any(|pattern| pattern.is_match(&p.text))
                {
                    return false;
                }
                true
            })
            .collect()
    }

    /// Process a single post
    async fn process_post(&self, post: &SourcePost, taxonomy: &Taxonomy) -> ProcessResult {
        // Check idempotency
        match self
            .state_store
            .is_processed(&post.id, &taxonomy.hash)
            .await
        {
            Ok(true) => {
                return ProcessResult::Skipped {
                    reason: "Already processed with this taxonomy".to_string(),
                };
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to check processed state, continuing");
            }
            Ok(false) => {}
        }

        // Classify
        let classify_usecase = ClassifyUseCase::new(
            self.classifier.as_ref(),
            self.config.classify_config.clone(),
        );

        let classification = match classify_usecase.classify(post, &taxonomy.definitions).await {
            Ok(c) => c,
            Err(e) => {
                return ProcessResult::Failed {
                    error: format!("Classification failed: {}", e),
                };
            }
        };

        tracing::info!(
            post_id = %post.id,
            tags = ?classification.tags.iter().map(|t| &t.id).collect::<Vec<_>>(),
            "Classified post"
        );

        if self.config.dry_run {
            let renderer = Renderer::new(self.config.render_config.clone());
            let rendered = renderer.render_for_x(post, &classification);
            tracing::info!(
                post_id = %post.id,
                rendered_text = %rendered.text,
                "[DRY RUN] Would publish"
            );
            return ProcessResult::Published {
                classification,
                x_post_id: None,
                nostr_event_id: None,
            };
        }

        // Publish
        let renderer = Renderer::new(self.config.render_config.clone());
        let mut x_post_id = None;
        let mut nostr_event_id = None;

        // Publish to X
        if self.x_publisher.is_enabled() {
            let rendered = renderer.render_for_x(post, &classification);
            match self.x_publisher.publish(&rendered).await {
                Ok(result) => {
                    x_post_id = Some(result.id);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to publish to X");
                }
            }
        }

        // Publish to Nostr
        if self.nostr_publisher.is_enabled() {
            let rendered = renderer.render_for_nostr(post, &classification);
            match self.nostr_publisher.publish(&rendered).await {
                Ok(result) => {
                    nostr_event_id = Some(result.id);
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to publish to Nostr");
                }
            }
        }

        // Record published state
        let record = PublishedRecord {
            id: Uuid::new_v4(),
            source_post_id: post.id.clone(),
            taxonomy_hash: taxonomy.hash.clone(),
            x_post_id: x_post_id.clone(),
            nostr_event_id: nostr_event_id.clone(),
            published_at: self.clock.now(),
        };

        if let Err(e) = self.state_store.record_published(&record).await {
            tracing::error!(error = %e, "Failed to record published state");
        }

        ProcessResult::Published {
            classification,
            x_post_id,
            nostr_event_id,
        }
    }
}

/// Errors from the run loop
#[derive(Debug, thiserror::Error)]
pub enum RunLoopError {
    #[error("Definitions error: {0}")]
    Definitions(String),
    #[error("Post source error: {0}")]
    PostSource(String),
    #[error("State error: {0}")]
    State(String),
}

#[derive(Debug)]
struct RateLimiter {
    per_minute: Option<u32>,
    per_hour: Option<u32>,
    state: Mutex<RateLimiterState>,
}

#[derive(Debug)]
struct RateLimiterState {
    minute_window_start: Instant,
    hour_window_start: Instant,
    minute_count: u32,
    hour_count: u32,
}

impl RateLimiter {
    fn new(per_minute: Option<u32>, per_hour: Option<u32>) -> Self {
        let now = Instant::now();
        Self {
            per_minute,
            per_hour,
            state: Mutex::new(RateLimiterState {
                minute_window_start: now,
                hour_window_start: now,
                minute_count: 0,
                hour_count: 0,
            }),
        }
    }

    async fn acquire(&self) {
        if self.per_minute.is_none() && self.per_hour.is_none() {
            return;
        }

        loop {
            let mut state = self.state.lock().await;
            let now = Instant::now();

            if now.duration_since(state.minute_window_start) >= Duration::from_secs(60) {
                state.minute_window_start = now;
                state.minute_count = 0;
            }

            if now.duration_since(state.hour_window_start) >= Duration::from_secs(3600) {
                state.hour_window_start = now;
                state.hour_count = 0;
            }

            let mut wait_for = Duration::from_secs(0);
            if let Some(limit) = self.per_minute {
                if state.minute_count >= limit {
                    let elapsed = now.duration_since(state.minute_window_start);
                    wait_for = wait_for.max(Duration::from_secs(60).saturating_sub(elapsed));
                }
            }

            if let Some(limit) = self.per_hour {
                if state.hour_count >= limit {
                    let elapsed = now.duration_since(state.hour_window_start);
                    wait_for = wait_for.max(Duration::from_secs(3600).saturating_sub(elapsed));
                }
            }

            if wait_for.is_zero() {
                if self.per_minute.is_some() {
                    state.minute_count = state.minute_count.saturating_add(1);
                }
                if self.per_hour.is_some() {
                    state.hour_count = state.hour_count.saturating_add(1);
                }
                return;
            }

            drop(state);
            sleep(wait_for).await;
        }
    }
}

fn compile_ignore_patterns(patterns: &[String]) -> Vec<Regex> {
    patterns
        .iter()
        .filter_map(|pattern| match Regex::new(pattern) {
            Ok(regex) => Some(regex),
            Err(error) => {
                tracing::warn!(pattern = %pattern, error = %error, "Invalid ignore pattern");
                None
            }
        })
        .collect()
}

// Implement Classifier for Arc<C> where C: Classifier
use async_trait::async_trait;

#[async_trait]
impl<C: Classifier + ?Sized> Classifier for &C {
    async fn classify(
        &self,
        input: crate::model::ClassifyInput,
    ) -> Result<crate::model::ClassifyOutput, ClassifyError> {
        (*self).classify(input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ClassifyInput, ClassifyOutput, RenderedPost, TagDefinition, TagMatch};
    use crate::ports::{
        DefinitionsError, PostSourceError, PublishError, PublishResult, StateError,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use time::OffsetDateTime;

    // Fake implementations for testing
    struct FakePostSource {
        posts: Vec<SourcePost>,
    }

    #[async_trait]
    impl PostSource for FakePostSource {
        async fn fetch_posts(
            &self,
            _account: &str,
            _since_id: Option<&str>,
        ) -> Result<Vec<SourcePost>, PostSourceError> {
            Ok(self.posts.clone())
        }
    }

    struct FakeDefinitionsRepo {
        definitions: Vec<TagDefinition>,
    }

    #[async_trait]
    impl DefinitionsRepo for FakeDefinitionsRepo {
        async fn load(&self) -> Result<Vec<TagDefinition>, DefinitionsError> {
            Ok(self.definitions.clone())
        }

        async fn validate(&self) -> Result<(), DefinitionsError> {
            Ok(())
        }
    }

    struct FakeClassifier;

    #[async_trait]
    impl Classifier for FakeClassifier {
        async fn classify(&self, _input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError> {
            Ok(ClassifyOutput::new(
                "Test summary".to_string(),
                vec![TagMatch {
                    id: "test_tag".to_string(),
                    confidence: 0.9,
                    rationale: "Test rationale".to_string(),
                    evidence: vec!["evidence".to_string()],
                }],
            ))
        }
    }

    struct FakePublisher {
        enabled: bool,
        platform: &'static str,
    }

    #[async_trait]
    impl Publisher for FakePublisher {
        async fn publish(&self, _post: &RenderedPost) -> Result<PublishResult, PublishError> {
            Ok(PublishResult {
                id: "fake_id".to_string(),
                url: None,
            })
        }

        fn is_enabled(&self) -> bool {
            self.enabled
        }

        fn platform(&self) -> &'static str {
            self.platform
        }
    }

    struct FakeStateStore {
        accounts: Mutex<HashMap<String, AccountState>>,
        processed: Mutex<HashMap<String, bool>>,
    }

    impl FakeStateStore {
        fn new() -> Self {
            Self {
                accounts: Mutex::new(HashMap::new()),
                processed: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl StateStore for FakeStateStore {
        async fn get_account_state(
            &self,
            account: &str,
        ) -> Result<Option<AccountState>, StateError> {
            Ok(self.accounts.lock().unwrap().get(account).cloned())
        }

        async fn set_account_state(&self, state: &AccountState) -> Result<(), StateError> {
            self.accounts
                .lock()
                .unwrap()
                .insert(state.account.clone(), state.clone());
            Ok(())
        }

        async fn is_processed(
            &self,
            source_post_id: &str,
            taxonomy_hash: &str,
        ) -> Result<bool, StateError> {
            let key = format!("{}:{}", source_post_id, taxonomy_hash);
            Ok(*self.processed.lock().unwrap().get(&key).unwrap_or(&false))
        }

        async fn record_published(&self, record: &PublishedRecord) -> Result<(), StateError> {
            let key = format!("{}:{}", record.source_post_id, record.taxonomy_hash);
            self.processed.lock().unwrap().insert(key, true);
            Ok(())
        }

        async fn get_published(
            &self,
            _source_post_id: &str,
            _taxonomy_hash: &str,
        ) -> Result<Option<PublishedRecord>, StateError> {
            Ok(None)
        }
    }

    struct FakeClock {
        time: OffsetDateTime,
    }

    impl Clock for FakeClock {
        fn now(&self) -> OffsetDateTime {
            self.time
        }
    }

    #[tokio::test]
    async fn test_poll_once_processes_posts() {
        let post_source = Arc::new(FakePostSource {
            posts: vec![SourcePost {
                id: "post1".to_string(),
                text: "Test post".to_string(),
                author: "testuser".to_string(),
                url: "https://x.com/testuser/status/post1".to_string(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            }],
        });

        let definitions_repo = Arc::new(FakeDefinitionsRepo {
            definitions: vec![TagDefinition {
                id: "test_tag".to_string(),
                title: "Test Tag".to_string(),
                aliases: vec![],
                short: None,
                content: "Test definition".to_string(),
                file_path: "test_tag.md".to_string(),
            }],
        });

        let classifier = Arc::new(FakeClassifier);
        let x_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "x",
        });
        let nostr_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "nostr",
        });
        let state_store = Arc::new(FakeStateStore::new());
        let clock = Arc::new(FakeClock {
            time: OffsetDateTime::now_utc(),
        });

        let config = RunLoopConfig {
            accounts: vec!["testuser".to_string()],
            dry_run: true,
            ..Default::default()
        };

        let run_loop = RunLoop::new(
            post_source,
            definitions_repo,
            classifier,
            x_publisher,
            nostr_publisher,
            state_store,
            clock,
            config,
        );

        let results = run_loop.poll_once().await.unwrap();

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].1, ProcessResult::Published { .. }));
    }

    #[tokio::test]
    async fn test_poll_once_filters_replies() {
        let post_source = Arc::new(FakePostSource {
            posts: vec![SourcePost {
                id: "reply1".to_string(),
                text: "This is a reply".to_string(),
                author: "testuser".to_string(),
                url: "https://x.com/testuser/status/reply1".to_string(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: true,
                reply_to_id: Some("original".to_string()),
            }],
        });

        let definitions_repo = Arc::new(FakeDefinitionsRepo {
            definitions: vec![],
        });
        let classifier = Arc::new(FakeClassifier);
        let x_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "x",
        });
        let nostr_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "nostr",
        });
        let state_store = Arc::new(FakeStateStore::new());
        let clock = Arc::new(FakeClock {
            time: OffsetDateTime::now_utc(),
        });

        let config = RunLoopConfig {
            accounts: vec!["testuser".to_string()],
            include_replies: false, // Filter out replies
            dry_run: true,
            ..Default::default()
        };

        let run_loop = RunLoop::new(
            post_source,
            definitions_repo,
            classifier,
            x_publisher,
            nostr_publisher,
            state_store,
            clock,
            config,
        );

        let results = run_loop.poll_once().await.unwrap();

        // Reply should be filtered out
        assert_eq!(results.len(), 0);
    }

    #[tokio::test]
    async fn test_poll_once_filters_ignore_patterns() {
        let post_source = Arc::new(FakePostSource {
            posts: vec![SourcePost {
                id: "ad1".to_string(),
                text: "AD: Buy now".to_string(),
                author: "testuser".to_string(),
                url: "https://x.com/testuser/status/ad1".to_string(),
                created_at: OffsetDateTime::now_utc(),
                is_repost: false,
                is_reply: false,
                reply_to_id: None,
            }],
        });

        let definitions_repo = Arc::new(FakeDefinitionsRepo {
            definitions: vec![],
        });
        let classifier = Arc::new(FakeClassifier);
        let x_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "x",
        });
        let nostr_publisher = Arc::new(FakePublisher {
            enabled: false,
            platform: "nostr",
        });
        let state_store = Arc::new(FakeStateStore::new());
        let clock = Arc::new(FakeClock {
            time: OffsetDateTime::now_utc(),
        });

        let config = RunLoopConfig {
            accounts: vec!["testuser".to_string()],
            ignore_patterns: vec!["^AD:".to_string()],
            dry_run: true,
            ..Default::default()
        };

        let run_loop = RunLoop::new(
            post_source,
            definitions_repo,
            classifier,
            x_publisher,
            nostr_publisher,
            state_store,
            clock,
            config,
        );

        let results = run_loop.poll_once().await.unwrap();

        // Post should be filtered out by ignore pattern
        assert_eq!(results.len(), 0);
    }
}
