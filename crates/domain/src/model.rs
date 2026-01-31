//! Domain models and value objects

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use uuid::Uuid;

/// A source post from a watched platform (e.g., X/Twitter)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourcePost {
    /// Platform-specific post ID
    pub id: String,
    /// Post text content
    pub text: String,
    /// Author username/handle
    pub author: String,
    /// URL to the original post
    pub url: String,
    /// When the post was created
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Whether this is a repost/retweet
    pub is_repost: bool,
    /// Whether this is a reply
    pub is_reply: bool,
    /// ID of post being replied to, if any
    pub reply_to_id: Option<String>,
}

/// A user-defined narrative tag definition loaded from markdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagDefinition {
    /// Unique identifier (from filename or frontmatter)
    pub id: String,
    /// Human-readable title
    pub title: String,
    /// Optional aliases for the tag
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Short description (from frontmatter)
    pub short: Option<String>,
    /// Full markdown content
    pub content: String,
    /// Source file path
    pub file_path: String,
}

/// A collection of tag definitions with computed hash
#[derive(Debug, Clone)]
pub struct Taxonomy {
    /// All loaded tag definitions
    pub definitions: Vec<TagDefinition>,
    /// SHA-256 hash of all definitions (for idempotency)
    pub hash: String,
}

impl Taxonomy {
    /// Create a new taxonomy from definitions, computing the hash
    pub fn new(mut definitions: Vec<TagDefinition>) -> Self {
        // Sort by ID for deterministic hashing
        definitions.sort_by(|a, b| a.id.cmp(&b.id));

        let mut hasher = Sha256::new();
        for def in &definitions {
            hasher.update(def.id.as_bytes());
            hasher.update(def.content.as_bytes());
            hasher.update(def.file_path.as_bytes());
        }
        let hash = format!("{:x}", hasher.finalize());

        Self { definitions, hash }
    }

    /// Get a definition by ID
    pub fn get(&self, id: &str) -> Option<&TagDefinition> {
        self.definitions.iter().find(|d| d.id == id)
    }

    /// Get all definition IDs
    pub fn ids(&self) -> Vec<&str> {
        self.definitions.iter().map(|d| d.id.as_str()).collect()
    }
}

/// Input for the classification use case
#[derive(Debug, Clone)]
pub struct ClassifyInput {
    /// The post to classify
    pub post: SourcePost,
    /// Tag definitions to consider
    pub definitions: Vec<TagDefinition>,
    /// Maximum output characters (for platform constraints)
    pub max_output_chars: Option<usize>,
    /// Optional policy/guardrails text
    pub policy_text: Option<String>,
}

/// A single tag match in classification output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagMatch {
    /// Tag ID that matched
    pub id: String,
    /// Confidence score 0.0-1.0
    pub confidence: f64,
    /// Rationale for the match
    pub rationale: String,
    /// Evidence excerpts from the post
    pub evidence: Vec<String>,
}

/// Output from the classification use case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifyOutput {
    /// Schema version
    pub version: String,
    /// Neutral summary of the post
    pub summary: String,
    /// Matched tags with confidences
    pub tags: Vec<TagMatch>,
}

impl ClassifyOutput {
    pub const SCHEMA_VERSION: &'static str = "1";

    /// Create a new classification output
    pub fn new(summary: String, tags: Vec<TagMatch>) -> Self {
        Self {
            version: Self::SCHEMA_VERSION.to_string(),
            summary,
            tags,
        }
    }
}

/// Publishing mode for X posts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum XPublishMode {
    /// Reply to the original post
    #[default]
    Reply,
    /// Quote the original post
    Quote,
    /// Create a standalone post with link
    NewPost,
}

/// Rendered content ready for publishing
#[derive(Debug, Clone)]
pub struct RenderedPost {
    /// The text content
    pub text: String,
    /// Reference to the original post
    pub source_post_id: String,
    /// Source post URL
    pub source_post_url: String,
}

/// Record of a published post (for idempotency)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishedRecord {
    /// Unique record ID
    pub id: Uuid,
    /// Source post ID
    pub source_post_id: String,
    /// Taxonomy hash at time of publishing
    pub taxonomy_hash: String,
    /// X post ID if published to X
    pub x_post_id: Option<String>,
    /// Nostr event ID if published to Nostr
    pub nostr_event_id: Option<String>,
    /// When published
    #[serde(with = "time::serde::rfc3339")]
    pub published_at: OffsetDateTime,
}

/// Account watch state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountState {
    /// Account username
    pub account: String,
    /// Last seen post ID
    pub since_id: Option<String>,
    /// When last updated
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Processing result for a single post
#[derive(Debug)]
pub enum ProcessResult {
    /// Post was classified and published
    Published {
        classification: ClassifyOutput,
        x_post_id: Option<String>,
        nostr_event_id: Option<String>,
    },
    /// Post was skipped (already processed, filtered, etc.)
    Skipped { reason: String },
    /// Classification or publishing failed
    Failed { error: String },
}
