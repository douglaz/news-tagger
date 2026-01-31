//! SQLite state store implementation

use async_trait::async_trait;
use news_tagger_domain::{AccountState, PublishedRecord, StateError, StateStore};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::path::Path;
use time::OffsetDateTime;
use uuid::Uuid;

/// SQLite-backed state store
pub struct SqliteStateStore {
    pool: SqlitePool,
}

impl SqliteStateStore {
    /// Create a new SQLite state store, initializing the database if needed
    pub async fn new(db_path: impl AsRef<Path>) -> Result<Self, StateError> {
        let db_path = db_path.as_ref();

        // Create parent directories if needed
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| StateError::Database(format!("Failed to create directory: {}", e)))?;
        }

        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&db_url)
            .await
            .map_err(|e| StateError::Database(e.to_string()))?;

        let store = Self { pool };
        store.run_migrations().await?;

        Ok(store)
    }

    /// Create an in-memory SQLite store (for testing)
    pub async fn in_memory() -> Result<Self, StateError> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .map_err(|e| StateError::Database(e.to_string()))?;

        let store = Self { pool };
        store.run_migrations().await?;

        Ok(store)
    }

    async fn run_migrations(&self) -> Result<(), StateError> {
        // Create tables
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS account_state (
                account TEXT PRIMARY KEY,
                since_id TEXT,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS published_records (
                id TEXT PRIMARY KEY,
                source_post_id TEXT NOT NULL,
                taxonomy_hash TEXT NOT NULL,
                x_post_id TEXT,
                nostr_event_id TEXT,
                published_at TEXT NOT NULL,
                UNIQUE(source_post_id, taxonomy_hash)
            )
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        // Create index for lookups
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_published_lookup
            ON published_records(source_post_id, taxonomy_hash)
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }
}

#[async_trait]
impl StateStore for SqliteStateStore {
    async fn get_account_state(&self, account: &str) -> Result<Option<AccountState>, StateError> {
        let row: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT account, since_id, updated_at FROM account_state WHERE account = ?",
        )
        .bind(account)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        match row {
            Some((account, since_id, updated_at_str)) => {
                let updated_at = OffsetDateTime::parse(
                    &updated_at_str,
                    &time::format_description::well_known::Rfc3339,
                )
                .map_err(|e| StateError::Serialization(e.to_string()))?;

                Ok(Some(AccountState {
                    account,
                    since_id,
                    updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn set_account_state(&self, state: &AccountState) -> Result<(), StateError> {
        let updated_at_str = state
            .updated_at
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| StateError::Serialization(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO account_state (account, since_id, updated_at)
            VALUES (?, ?, ?)
            ON CONFLICT(account) DO UPDATE SET
                since_id = excluded.since_id,
                updated_at = excluded.updated_at
            "#,
        )
        .bind(&state.account)
        .bind(&state.since_id)
        .bind(&updated_at_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    async fn is_processed(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<bool, StateError> {
        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM published_records WHERE source_post_id = ? AND taxonomy_hash = ?",
        )
        .bind(source_post_id)
        .bind(taxonomy_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        Ok(count.0 > 0)
    }

    async fn record_published(&self, record: &PublishedRecord) -> Result<(), StateError> {
        let published_at_str = record
            .published_at
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| StateError::Serialization(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO published_records
            (id, source_post_id, taxonomy_hash, x_post_id, nostr_event_id, published_at)
            VALUES (?, ?, ?, ?, ?, ?)
            ON CONFLICT(source_post_id, taxonomy_hash) DO UPDATE SET
                x_post_id = COALESCE(excluded.x_post_id, published_records.x_post_id),
                nostr_event_id = COALESCE(excluded.nostr_event_id, published_records.nostr_event_id)
            "#,
        )
        .bind(record.id.to_string())
        .bind(&record.source_post_id)
        .bind(&record.taxonomy_hash)
        .bind(&record.x_post_id)
        .bind(&record.nostr_event_id)
        .bind(&published_at_str)
        .execute(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        Ok(())
    }

    async fn get_published(
        &self,
        source_post_id: &str,
        taxonomy_hash: &str,
    ) -> Result<Option<PublishedRecord>, StateError> {
        let row: Option<(
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        )> = sqlx::query_as(
            r#"
                SELECT id, source_post_id, taxonomy_hash, x_post_id, nostr_event_id, published_at
                FROM published_records
                WHERE source_post_id = ? AND taxonomy_hash = ?
                "#,
        )
        .bind(source_post_id)
        .bind(taxonomy_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StateError::Database(e.to_string()))?;

        match row {
            Some((
                id,
                source_post_id,
                taxonomy_hash,
                x_post_id,
                nostr_event_id,
                published_at_str,
            )) => {
                let id =
                    Uuid::parse_str(&id).map_err(|e| StateError::Serialization(e.to_string()))?;

                let published_at = OffsetDateTime::parse(
                    &published_at_str,
                    &time::format_description::well_known::Rfc3339,
                )
                .map_err(|e| StateError::Serialization(e.to_string()))?;

                Ok(Some(PublishedRecord {
                    id,
                    source_post_id,
                    taxonomy_hash,
                    x_post_id,
                    nostr_event_id,
                    published_at,
                }))
            }
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_account_state_roundtrip() {
        let store = SqliteStateStore::in_memory().await.unwrap();

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
    async fn test_published_record_roundtrip() {
        let store = SqliteStateStore::in_memory().await.unwrap();

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
    async fn test_upsert_account_state() {
        let store = SqliteStateStore::in_memory().await.unwrap();

        let state1 = AccountState {
            account: "testuser".to_string(),
            since_id: Some("111".to_string()),
            updated_at: OffsetDateTime::now_utc(),
        };
        store.set_account_state(&state1).await.unwrap();

        let state2 = AccountState {
            account: "testuser".to_string(),
            since_id: Some("222".to_string()),
            updated_at: OffsetDateTime::now_utc(),
        };
        store.set_account_state(&state2).await.unwrap();

        let retrieved = store.get_account_state("testuser").await.unwrap().unwrap();
        assert_eq!(retrieved.since_id, Some("222".to_string()));
    }
}
