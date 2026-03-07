//! JSONL file-based post source adapter

use async_trait::async_trait;
use news_tagger_domain::{PostSource, PostSourceError, SourcePost};
use std::path::PathBuf;

/// Post source that reads SourcePost entries from a JSONL file
pub struct JsonlPostSource {
    paths: Vec<PathBuf>,
}

impl JsonlPostSource {
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }

    fn load_posts(&self) -> Result<Vec<SourcePost>, PostSourceError> {
        let mut posts = Vec::new();
        for path in &self.paths {
            let content = std::fs::read_to_string(path).map_err(|e| {
                PostSourceError::Api(format!("Failed to read {}: {}", path.display(), e))
            })?;
            for (i, line) in content.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let post: SourcePost = serde_json::from_str(line).map_err(|e| {
                    PostSourceError::Api(format!(
                        "Failed to parse line {} in {}: {}",
                        i + 1,
                        path.display(),
                        e
                    ))
                })?;
                posts.push(post);
            }
        }
        posts.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(posts)
    }
}

#[async_trait]
impl PostSource for JsonlPostSource {
    async fn fetch_posts(
        &self,
        account: &str,
        since_id: Option<&str>,
    ) -> Result<Vec<SourcePost>, PostSourceError> {
        let posts = self.load_posts()?;
        let filtered: Vec<SourcePost> = posts
            .into_iter()
            .filter(|p| account == "*" || p.author.eq_ignore_ascii_case(account))
            .filter(|p| since_id.map(|sid| p.id.as_str() > sid).unwrap_or(true))
            .collect();
        Ok(filtered)
    }
}
