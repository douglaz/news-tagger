//! Filesystem-based definitions repository

use async_trait::async_trait;
use news_tagger_domain::{DefinitionsError, DefinitionsRepo, TagDefinition};
use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Filesystem definitions repository
pub struct FsDefinitionsRepo {
    definitions_dir: PathBuf,
    id_pattern: Regex,
}

impl FsDefinitionsRepo {
    /// Create a new filesystem definitions repo
    pub fn new(definitions_dir: impl AsRef<Path>) -> Result<Self, DefinitionsError> {
        let definitions_dir = definitions_dir.as_ref().to_path_buf();

        if !definitions_dir.exists() {
            return Err(DefinitionsError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "Definitions directory not found: {}",
                    definitions_dir.display()
                ),
            )));
        }

        let id_pattern = Regex::new(r"^[a-z0-9_]+$").expect("Valid regex");

        Ok(Self {
            definitions_dir,
            id_pattern,
        })
    }

    /// Parse frontmatter from markdown content
    fn parse_frontmatter(&self, content: &str) -> (Option<Frontmatter>, String) {
        if !content.starts_with("---") {
            return (None, content.to_string());
        }

        let parts: Vec<&str> = content.splitn(3, "---").collect();
        if parts.len() < 3 {
            return (None, content.to_string());
        }

        let frontmatter_str = parts[1].trim();
        let body = parts[2].trim().to_string();

        // Simple YAML-like parsing (not full YAML)
        let frontmatter = self.parse_simple_yaml(frontmatter_str);

        (Some(frontmatter), body)
    }

    /// Simple YAML-like frontmatter parser
    fn parse_simple_yaml(&self, yaml: &str) -> Frontmatter {
        let mut fm = Frontmatter::default();

        for line in yaml.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim();
                let value = value.trim().trim_matches('"').trim_matches('\'');

                match key {
                    "id" => fm.id = Some(value.to_string()),
                    "title" => fm.title = Some(value.to_string()),
                    "short" => fm.short = Some(value.to_string()),
                    "aliases" => {
                        // Handle inline array: [a, b, c]
                        if value.starts_with('[') && value.ends_with(']') {
                            fm.aliases = value[1..value.len() - 1]
                                .split(',')
                                .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                    }
                    _ => {}
                }
            }
        }

        fm
    }

    /// Extract title from first H1 in markdown
    fn extract_title_from_markdown(&self, content: &str) -> Option<String> {
        for line in content.lines() {
            let line = line.trim();
            if let Some(stripped) = line.strip_prefix("# ") {
                return Some(stripped.trim().to_string());
            }
        }
        None
    }

    /// Validate a tag ID
    fn validate_id(&self, id: &str) -> Result<(), DefinitionsError> {
        if !self.id_pattern.is_match(id) {
            return Err(DefinitionsError::InvalidId { id: id.to_string() });
        }
        Ok(())
    }
}

#[derive(Default)]
struct Frontmatter {
    id: Option<String>,
    title: Option<String>,
    short: Option<String>,
    aliases: Vec<String>,
}

#[async_trait]
impl DefinitionsRepo for FsDefinitionsRepo {
    async fn load(&self) -> Result<Vec<TagDefinition>, DefinitionsError> {
        let mut definitions = Vec::new();
        let mut ids_seen: HashMap<String, String> = HashMap::new();

        let entries = std::fs::read_dir(&self.definitions_dir)?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let extension = path.extension().and_then(|e| e.to_str());
            if extension != Some("md") {
                continue;
            }

            let file_stem = path.file_stem().and_then(|s| s.to_str()).ok_or_else(|| {
                DefinitionsError::Parse {
                    file: path.display().to_string(),
                    message: "Invalid filename".to_string(),
                }
            })?;

            let content = std::fs::read_to_string(&path)?;
            let (frontmatter, body) = self.parse_frontmatter(&content);

            let id = frontmatter
                .as_ref()
                .and_then(|f| f.id.clone())
                .unwrap_or_else(|| file_stem.to_string());

            self.validate_id(&id)?;

            // Check for duplicates
            if let Some(existing_file) = ids_seen.get(&id) {
                return Err(DefinitionsError::DuplicateId {
                    id,
                    files: vec![existing_file.clone(), path.display().to_string()],
                });
            }
            ids_seen.insert(id.clone(), path.display().to_string());

            let title = frontmatter
                .as_ref()
                .and_then(|f| f.title.clone())
                .or_else(|| self.extract_title_from_markdown(&body))
                .unwrap_or_else(|| id.replace('_', " "));

            let definition = TagDefinition {
                id,
                title,
                aliases: frontmatter
                    .as_ref()
                    .map(|f| f.aliases.clone())
                    .unwrap_or_default(),
                short: frontmatter.and_then(|f| f.short),
                content,
                file_path: path.display().to_string(),
            };

            definitions.push(definition);
        }

        if definitions.is_empty() {
            return Err(DefinitionsError::Empty(
                self.definitions_dir.display().to_string(),
            ));
        }

        // Sort by ID for deterministic ordering
        definitions.sort_by(|a, b| a.id.cmp(&b.id));

        Ok(definitions)
    }

    async fn validate(&self) -> Result<(), DefinitionsError> {
        // Load will perform validation
        let _ = self.load().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[tokio::test]
    async fn test_load_simple_definition() {
        let dir = setup_test_dir();
        let def_path = dir.path().join("test_tag.md");
        std::fs::write(&def_path, "# Test Tag\n\nSome content here.").unwrap();

        let repo = FsDefinitionsRepo::new(dir.path()).unwrap();
        let definitions = repo.load().await.unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].id, "test_tag");
        assert_eq!(definitions[0].title, "Test Tag");
    }

    #[tokio::test]
    async fn test_load_with_frontmatter() {
        let dir = setup_test_dir();
        let content = r#"---
id: custom_id
title: Custom Title
short: A short description
aliases: [alias1, alias2]
---
# Custom Title

Full content here.
"#;
        std::fs::write(dir.path().join("whatever.md"), content).unwrap();

        let repo = FsDefinitionsRepo::new(dir.path()).unwrap();
        let definitions = repo.load().await.unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].id, "custom_id");
        assert_eq!(definitions[0].title, "Custom Title");
        assert_eq!(definitions[0].short.as_deref(), Some("A short description"));
        assert_eq!(definitions[0].aliases, vec!["alias1", "alias2"]);
    }

    #[tokio::test]
    async fn test_duplicate_id_error() {
        let dir = setup_test_dir();
        std::fs::write(dir.path().join("tag_a.md"), "# Tag A").unwrap();
        std::fs::write(dir.path().join("tag_b.md"), "---\nid: tag_a\n---\n# Tag B").unwrap();

        let repo = FsDefinitionsRepo::new(dir.path()).unwrap();
        let result = repo.load().await;

        assert!(matches!(result, Err(DefinitionsError::DuplicateId { .. })));
    }

    #[tokio::test]
    async fn test_invalid_id_error() {
        let dir = setup_test_dir();
        std::fs::write(dir.path().join("Invalid-Tag.md"), "# Invalid").unwrap();

        let repo = FsDefinitionsRepo::new(dir.path()).unwrap();
        let result = repo.load().await;

        assert!(matches!(result, Err(DefinitionsError::InvalidId { .. })));
    }

    #[tokio::test]
    async fn test_empty_directory_error() {
        let dir = setup_test_dir();

        let repo = FsDefinitionsRepo::new(dir.path()).unwrap();
        let result = repo.load().await;

        assert!(matches!(result, Err(DefinitionsError::Empty(_))));
    }

    #[tokio::test]
    async fn test_nonexistent_directory() {
        let result = FsDefinitionsRepo::new("/nonexistent/path");
        assert!(result.is_err());
    }
}
