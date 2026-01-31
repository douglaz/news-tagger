//! Port definitions (traits) for external dependencies
//!
//! These traits define the boundaries between the domain and external systems.
//! Adapters implement these traits to connect to real infrastructure.

use async_trait::async_trait;
use thiserror::Error;
use time::OffsetDateTime;

use crate::model::{
    AccountState, ClassifyInput, ClassifyOutput, PublishedRecord, RenderedPost, SourcePost,
    TagDefinition,
};

/// Error type for post source operations
#[derive(Debug, Error)]
pub enum PostSourceError {
    #[error("API error: {0}")]
    Api(String),
    #[error("Rate limited, retry after: {0:?}")]
    RateLimited(Option<std::time::Duration>),
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Network error: {0}")]
    Network(String),
}

/// Port for fetching posts from a source platform
#[async_trait]
pub trait PostSource: Send + Sync {
    /// Fetch posts for an account since the given ID
    async fn fetch_posts(
        &self,
        account: &str,
        since_id: Option<&str>,
    ) -> Result<Vec<SourcePost>, PostSourceError>;
}

/// Error type for classifier operations
#[derive(Debug, Error)]
pub enum ClassifyError {
    #[error("LLM API error: {0}")]
    Api(String),
    #[error("Invalid response format: {0}")]
    InvalidFormat(String),
    #[error("Rate limited")]
    RateLimited,
    #[error("Timeout")]
    Timeout,
    #[error("Configuration error: {0}")]
    Config(String),
}

/// Port for LLM-based classification
#[async_trait]
pub trait Classifier: Send + Sync {
    /// Classify a post against the provided definitions
    async fn classify(&self, input: ClassifyInput) -> Result<ClassifyOutput, ClassifyError>;
}

/// Error type for publisher operations
#[derive(Debug, Error)]
pub enum PublishError {
    #[error("API error: {0}")]
    Api(String),
    #[error("Rate limited")]
    RateLimited,
    #[error("Authentication failed: {0}")]
    Auth(String),
    #[error("Content too long: {len} > {max}")]
    ContentTooLong { len: usize, max: usize },
}

/// Result of a successful publish operation
#[derive(Debug, Clone)]
pub struct PublishResult {
    /// Platform-specific post/event ID
    pub id: String,
    /// URL to the published content, if available
    pub url: Option<String>,
}

/// Port for publishing classification results
#[async_trait]
pub trait Publisher: Send + Sync {
    /// Publish a rendered post, returns the published ID
    async fn publish(&self, post: &RenderedPost) -> Result<PublishResult, PublishError>;

    /// Check if this publisher is enabled
    fn is_enabled(&self) -> bool;

    /// Get the platform name (e.g., "x", "nostr")
    fn platform(&self) -> &'static str;
}

/// Error type for definitions repository
#[derive(Debug, Error)]
pub enum DefinitionsError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Parse error in {file}: {message}")]
    Parse { file: String, message: String },
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("No definitions found in {0}")]
    Empty(String),
    #[error("Duplicate ID '{id}' in files: {files:?}")]
    DuplicateId { id: String, files: Vec<String> },
    #[error("Invalid ID '{id}': must match [a-z0-9_]+")]
    InvalidId { id: String },
}

/// Port for loading tag definitions
#[async_trait]
pub trait DefinitionsRepo: Send + Sync {
    /// Load all definitions from the configured source
    async fn load(&self) -> Result<Vec<TagDefinition>, DefinitionsError>;

    /// Validate definitions without loading fully
    async fn validate(&self) -> Result<(), DefinitionsError>;
}

/// Error type for state store operations
#[derive(Debug, Error)]
pub enum StateError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
}

/// Port for persisting application state
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Get account state (since_id, etc.)
    async fn get_account_state(&self, account: &str) -> Result<Option<AccountState>, StateError>;

    /// Update account state
    async fn set_account_state(&self, state: &AccountState) -> Result<(), StateError>;

    /// Check if a post has been processed with the given taxonomy
    async fn is_processed(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<bool, StateError>;

    /// Record a published post
    async fn record_published(&self, record: &PublishedRecord) -> Result<(), StateError>;

    /// Get published record for a source post
    async fn get_published(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<Option<PublishedRecord>, StateError>;
}

/// Port for time/clock operations (enables deterministic testing)
pub trait Clock: Send + Sync {
    /// Get the current time
    fn now(&self) -> OffsetDateTime;
}

/// Real clock implementation
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}
