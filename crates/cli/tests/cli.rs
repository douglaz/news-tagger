use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn write_definition(dir: &TempDir, name: &str, id: &str, title: &str) {
    let content = format!(
        "---\nid: {}\ntitle: {}\n---\n\n# {}\n\nDefinition content.\n",
        id, title, title
    );
    let path = dir.path().join(name);
    fs::write(&path, content).expect("write definition");
}

#[test]
fn config_init_writes_example_file() {
    let dir = TempDir::new().expect("temp dir");
    let config_path = dir.path().join("config.toml");

    let mut cmd = cargo_bin_cmd!("news-tagger");
    cmd.args(["config", "init", "--path"])
        .arg(&config_path)
        .assert()
        .success();

    let content = fs::read_to_string(&config_path).expect("read config");
    assert!(content.contains("definitions_dir"));
    assert!(content.contains("dry_run = true"));
}

#[test]
fn definitions_validate_fails_on_duplicate_ids() {
    let dir = TempDir::new().expect("temp dir");
    write_definition(&dir, "a.md", "duplicate_id", "First");
    write_definition(&dir, "b.md", "duplicate_id", "Second");

    let mut cmd = cargo_bin_cmd!("news-tagger");
    cmd.args(["definitions", "validate", "--definitions-dir"])
        .arg(dir.path())
        .assert()
        .failure()
        .stderr(predicate::str::contains("Validation failed"));
}

#[test]
fn classify_outputs_valid_json() {
    let dir = TempDir::new().expect("temp dir");
    write_definition(
        &dir,
        "example.md",
        "example_narrative",
        "Example Narrative Tag",
    );

    let mut cmd = cargo_bin_cmd!("news-tagger");
    let output = cmd
        .env("NEWS_TAGGER__LLM__PROVIDER", "stub")
        .args([
            "classify",
            "--text",
            "This mentions example narrative tag in the post",
            "--definitions-dir",
        ])
        .arg(dir.path())
        .arg("--json")
        .output()
        .expect("run classify");

    assert!(output.status.success());

    let value: Value = serde_json::from_slice(&output.stdout).expect("valid json");
    assert!(value.get("summary").is_some());
    assert!(value.get("tags").is_some());
}
