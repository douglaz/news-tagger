//! news-tagger domain crate
//!
//! This crate contains the core domain logic following hexagonal architecture:
//! - `model`: Domain entities and value objects
//! - `ports`: Trait definitions for external dependencies (adapters)
//! - `usecases`: Application use cases / business logic
//! - `policy`: Safety and format constraints

pub mod model;
pub mod policy;
pub mod ports;
pub mod usecases;

pub use model::*;
pub use ports::*;

use sha2::{Digest, Sha256};

/// Compute a deterministic hash of a set of tag definitions
/// Used for idempotency checks
pub fn compute_taxonomy_hash(definitions: &[TagDefinition]) -> String {
    let mut sorted: Vec<_> = definitions.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));

    let mut hasher = Sha256::new();
    for def in sorted {
        hasher.update(def.id.as_bytes());
        hasher.update(def.content.as_bytes());
        hasher.update(def.file_path.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}
