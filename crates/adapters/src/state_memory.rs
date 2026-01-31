//! In-memory state store for testing and offline mode

use async_trait::async_trait;
use news_tagger_domain::{AccountState, PublishedRecord, StateError, StateStore};
use std::collections::HashMap;
use std::sync::RwLock;

/// In-memory state store implementation
pub struct InMemoryStateStore {
    accounts: RwLock<HashMap<String, AccountState>>,
    published: RwLock<HashMap<String, PublishedRecord>>,
}

impl InMemoryStateStore {
    pub fn new() -> Self {
        Self {
            accounts: RwLock::new(HashMap::new()),
            published: RwLock::new(HashMap::new()),
        }
    }

    fn make_published_key(source_post_id: &str, taxonomy_hash: &str) -> String {
        format!("{}:{}", source_post_id, taxonomy_hash)
    }
}

impl Default for InMemoryStateStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl StateStore for InMemoryStateStore {
    async fn get_account_state(&self, account: &str) -> Result<Option<AccountState>, StateError> {
        let accounts = self
            .accounts
            .read()
            .map_err(|e| StateError::Database(e.to_string()))?;
        Ok(accounts.get(account).cloned())
    }

    async fn set_account_state(&self, state: &AccountState) -> Result<(), StateError> {
        let mut accounts = self
            .accounts
            .write()
            .map_err(|e| StateError::Database(e.to_string()))?;
        accounts.insert(state.account.clone(), state.clone());
        Ok(())
    }

    async fn is_processed(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<bool, StateError> {
        let key = Self::make_published_key(source_post_id, taxonomy_hash);
        let published = self
            .published
            .read()
            .map_err(|e| StateError::Database(e.to_string()))?;
        Ok(published.contains_key(&key))
    }

    async fn record_published(&self, record: &PublishedRecord) -> Result<(), StateError> {
        let key = Self::make_published_key(&record.source_post_id, &record.taxonomy_hash);
        let mut published = self
            .published
            .write()
            .map_err(|e| StateError::Database(e.to_string()))?;
        published.insert(key, record.clone());
        Ok(())
    }

    async fn get_published(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<Option<PublishedRecord>, StateError> {
        let key = Self::make_published_key(source_post_id, taxonomy_hash);
        let published = self
            .published
            .read()
            .map_err(|e| StateError::Database(e.to_string()))?;
        Ok(published.get(&key).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_account_state_roundtrip() {
        let store = InMemoryStateStore::new();

        let state = AccountState {
            account: "testuser".to_string(),
            since_id: Some("12345".to_string()),
            updated_at: OffsetDateTime::now_utc(),
        };

        store.set_account_state(&state).await.unwrap();
        let retrieved = store.get_account_state("testuser").await.unwrap();

        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().since_id, Some("12345".to_string()));
    }

    #[tokio::test]
    async fn test_get_nonexistent_account() {
        let store = InMemoryStateStore::new();
        let result = store.get_account_state("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_published_record_roundtrip() {
        let store = InMemoryStateStore::new();

        let record = PublishedRecord {
            id: Uuid::new_v4(),
            source_post_id: "post123".to_string(),
            taxonomy_hash: "hash456".to_string(),
            x_post_id: Some("xpost789".to_string()),
            nostr_event_id: None,
            published_at: OffsetDateTime::now_utc(),
        };

        store.record_published(&record).await.unwrap();

        let is_processed = store.is_processed("post123", "hash456").await.unwrap();
        assert!(is_processed);

        let retrieved = store.get_published("post123", "hash456").await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().x_post_id, Some("xpost789".to_string()));
    }

    #[tokio::test]
    async fn test_not_processed_with_different_hash() {
        let store = InMemoryStateStore::new();

        let record = PublishedRecord {
            id: Uuid::new_v4(),
            source_post_id: "post123".to_string(),
            taxonomy_hash: "hash456".to_string(),
            x_post_id: None,
            nostr_event_id: None,
            published_at: OffsetDateTime::now_utc(),
        };

        store.record_published(&record).await.unwrap();

        // Same post but different taxonomy hash should not be processed
        let is_processed = store
            .is_processed("post123", "different_hash")
            .await
            .unwrap();
        assert!(!is_processed);
    }
}
