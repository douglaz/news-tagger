//! news-tagger adapters crate
//!
//! This crate contains infrastructure adapters implementing the domain ports:
//! - `definitions`: Filesystem-based definitions loader
//! - `state`: SQLite and in-memory state stores
//! - `llm`: LLM provider adapters (OpenAI, Anthropic, etc.)
//! - `x`: X (Twitter) API adapters
//! - `nostr`: Nostr publishing adapter
//! - `jsonl_source`: JSONL file-based post source

mod definitions_fs;
mod jsonl_source;
pub mod outbox;
mod state_memory;
mod state_sqlite;

pub mod llm;
pub mod nostr;
pub mod x_api;

/// Re-exports for definitions adapters
pub mod definitions {
    pub use crate::definitions_fs::FsDefinitionsRepo as FilesystemDefinitionsRepo;
}

/// Re-exports for state adapters
pub mod state {
    pub use crate::state_memory::InMemoryStateStore;
    pub use crate::state_sqlite::SqliteStateStore;
}

/// Re-exports for X API adapters
pub mod x {
    pub use crate::x_api::{StubPostSource, StubXPublisher, XPostSource, XPublisher};
}

/// Re-exports for file-based post source
pub mod jsonl {
    pub use crate::jsonl_source::JsonlPostSource;
}
