# news-tagger

A CLI tool for classifying social media posts using LLM-powered narrative tagging.

## Overview

`news-tagger` watches X (Twitter) accounts for new posts, classifies them against user-defined narrative tags using LLM providers, and optionally publishes the classification results back to X and/or Nostr.

**Key features:**

- **User-defined taxonomy**: Define narrative tags as markdown files
- **Multi-provider LLM support**: OpenAI, Anthropic, Gemini, Ollama, Claude Code CLI, Codex CLI, OpenCode server, or any OpenAI-compatible API
- **Multi-platform publishing**: X (Twitter) and Nostr
- **Idempotent & resumable**: Tracks processed posts to avoid duplicates
- **Offline testable**: Full test coverage without network calls

## Installation

### With Nix (Recommended)

```bash
nix develop
cargo build --release
```

### Without Nix

Requires Rust 1.85+ (for Rust 2024 edition).

```bash
cargo build --release
```

## Quick Start

1. **Initialize configuration:**

   ```bash
   news-tagger config init
   ```

2. **Create tag definitions:**

   Create a `definitions/` directory with markdown files:

   ```markdown
   ---
   id: fear_narrative
   title: Fear-based Narrative
   short: Content using fear appeals to drive engagement or action.
   ---
   # Fear-based Narrative

   Posts that use catastrophic framing, urgency without evidence, or
   appeal to fear to motivate action or engagement.
   ```

3. **Validate your setup:**

   ```bash
   news-tagger doctor
   ```

4. **Test classification:**

   ```bash
   news-tagger classify --text "Breaking: Experts warn of imminent disaster!" --json
   ```

5. **Run the watcher (dry-run first):**

   ```bash
   news-tagger run --dry-run --once
   ```

## CLI Commands

### `run`

Watch accounts, classify posts, and publish results.

```bash
news-tagger run [--dry-run] [--once] [--require-approval]
```

- `--dry-run`: Don't actually publish, just log what would happen
- `--once`: Process one poll cycle and exit
- `--require-approval`: Write to outbox file instead of publishing

### `classify`

One-shot classification of text.

```bash
news-tagger classify --text "Post content here"
news-tagger classify --file post.txt
echo "Post content" | news-tagger classify
news-tagger classify --json  # Output raw JSON
```

### `definitions`

Manage tag definitions.

```bash
news-tagger definitions list        # List all definitions
news-tagger definitions validate    # Validate definitions
```

### `config init`

Generate example configuration file.

```bash
news-tagger config init [--path config.toml] [--force]
```

### `doctor`

Validate configuration and show status.

```bash
news-tagger doctor [--json]
```

## Configuration

Configuration is loaded from:
1. `--config path` CLI argument
2. `./config.toml` (default)
3. Environment variables with `NEWS_TAGGER__` prefix

Example `config.toml`:

```toml
[general]
definitions_dir = "./definitions"
state_db_path = "./state.sqlite"
dry_run = true

[watch]
poll_interval_secs = 60
accounts = ["account1", "account2"]

[llm]
provider = "openai"
model = "gpt-4o-mini"
temperature = 0.2
retries = 2

[llm.openai]
api_key_env = "OPENAI_API_KEY"

[llm.claude_code]
command = "claude"
args = []

[llm.codex]
command = "codex"
args = []

[llm.opencode]
base_url = "http://127.0.0.1:4096"
provider_id = "opencode"
model_id = "big-pickle"

[x.read]
bearer_token_env = "X_BEARER_TOKEN"

[x.write]
enabled = false
mode = "reply"

[nostr]
enabled = false
relays = ["wss://relay.damus.io"]
```

## Tag Definition Format

Each `.md` file in the definitions directory becomes a tag. The tag ID defaults to the filename (without extension).

Optional YAML frontmatter:

```markdown
---
id: custom_id           # Override filename-based ID
title: Display Title    # Human-readable title
aliases:                # Alternative names
  - alias one
  - alias two
short: Brief description for output rendering.
---

# Full Definition

Detailed description of what this narrative tag represents...
```

## Architecture

This project uses hexagonal (ports & adapters) architecture:

```
crates/
  domain/      # Core business logic, ports (traits), models
  adapters/    # External system implementations
  cli/         # Command-line interface
```

## Development

```bash
# Enter dev shell
nix develop

# Run tests
cargo test --all

# Format code
cargo fmt --all

# Lint
cargo clippy --all-targets --all-features -D warnings

# Run CLI
cargo run -- --help
```

## Testing

All tests run offline without network calls:

- Domain tests use fake/stub implementations
- Adapter tests use `wiremock` for HTTP mocking
- CLI tests use `assert_cmd`

```bash
cargo test --all
```

## License

MIT OR Apache-2.0
