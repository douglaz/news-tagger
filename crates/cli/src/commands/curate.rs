//! Curate command - interactive TUI for manual post tagging

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use news_tagger_adapters::definitions::FilesystemDefinitionsRepo;
use news_tagger_domain::{DefinitionsRepo, SourcePost, TagDefinition};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;

use crate::args::CurateArgs;
use crate::config::AppConfig;

#[derive(Serialize, Deserialize)]
struct CuratedPost {
    post_id: String,
    author: String,
    text: String,
    url: String,
    tags: Vec<String>,
}

enum Mode {
    Normal,
    NewTag,
}

enum InputField {
    Id,
    Title,
    Short,
    Description,
}

struct NewTagDraft {
    id: String,
    title: String,
    short: String,
    description: String,
}

impl NewTagDraft {
    fn new() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            short: String::new(),
            description: String::new(),
        }
    }

    fn clear(&mut self) {
        self.id.clear();
        self.title.clear();
        self.short.clear();
        self.description.clear();
    }
}

struct App {
    posts: Vec<SourcePost>,
    definitions: Vec<TagDefinition>,
    current_post: usize,
    selected_tag: usize,
    tag_selections: HashSet<String>,
    curated_ids: HashSet<String>,
    mode: Mode,
    input_field: InputField,
    new_tag: NewTagDraft,
    output_path: PathBuf,
    definitions_dir: PathBuf,
    message: Option<String>,
    should_quit: bool,
    tag_scroll_offset: usize,
}

impl App {
    fn new(
        posts: Vec<SourcePost>,
        definitions: Vec<TagDefinition>,
        curated_ids: HashSet<String>,
        output_path: PathBuf,
        definitions_dir: PathBuf,
    ) -> Self {
        let current_post = posts
            .iter()
            .position(|p| !curated_ids.contains(&p.id))
            .unwrap_or(0);

        Self {
            posts,
            definitions,
            current_post,
            selected_tag: 0,
            tag_selections: HashSet::new(),
            curated_ids,
            mode: Mode::Normal,
            input_field: InputField::Id,
            new_tag: NewTagDraft::new(),
            output_path,
            definitions_dir,
            message: None,
            should_quit: false,
            tag_scroll_offset: 0,
        }
    }

    fn current_post(&self) -> Option<&SourcePost> {
        self.posts.get(self.current_post)
    }

    fn remaining(&self) -> usize {
        self.posts
            .iter()
            .skip(self.current_post)
            .filter(|p| !self.curated_ids.contains(&p.id))
            .count()
    }

    fn save_current(&mut self) -> Result<()> {
        if let Some(post) = self.posts.get(self.current_post) {
            let mut tags: Vec<String> = self.tag_selections.iter().cloned().collect();
            tags.sort();
            let curated = CuratedPost {
                post_id: post.id.clone(),
                author: post.author.clone(),
                text: post.text.clone(),
                url: post.url.clone(),
                tags,
            };
            let line = serde_json::to_string(&curated)?;
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.output_path)?;
            writeln!(file, "{}", line)?;
            self.curated_ids.insert(post.id.clone());
            let tag_count = self.tag_selections.len();
            self.message = Some(format!("Saved with {} tag(s)", tag_count));
        }
        self.tag_selections.clear();
        self.advance_to_next();
        Ok(())
    }

    fn skip_current(&mut self) {
        self.message = Some("Skipped".to_string());
        self.tag_selections.clear();
        self.advance_to_next();
    }

    fn advance_to_next(&mut self) {
        for i in (self.current_post + 1)..self.posts.len() {
            if !self.curated_ids.contains(&self.posts[i].id) {
                self.current_post = i;
                self.selected_tag = 0;
                self.tag_scroll_offset = 0;
                return;
            }
        }
        self.should_quit = true;
        self.message = Some("All posts curated!".to_string());
    }

    fn go_prev(&mut self) {
        if self.current_post > 0 {
            for i in (0..self.current_post).rev() {
                if !self.curated_ids.contains(&self.posts[i].id) {
                    self.current_post = i;
                    self.tag_selections.clear();
                    self.selected_tag = 0;
                    self.tag_scroll_offset = 0;
                    return;
                }
            }
        }
    }

    fn toggle_tag(&mut self) {
        if let Some(def) = self.definitions.get(self.selected_tag) {
            let id = def.id.clone();
            if self.tag_selections.contains(&id) {
                self.tag_selections.remove(&id);
            } else {
                self.tag_selections.insert(id);
            }
        }
    }

    fn create_tag(&mut self) -> Result<()> {
        let id = self.new_tag.id.trim().to_string();
        let title_input = self.new_tag.title.trim().to_string();
        let short = self.new_tag.short.trim().to_string();
        let description = self.new_tag.description.trim().to_string();

        if id.is_empty() {
            self.message = Some("Tag ID cannot be empty".to_string());
            return Ok(());
        }

        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            self.message = Some("ID must be lowercase letters, digits, underscores".to_string());
            return Ok(());
        }

        if self.definitions.iter().any(|d| d.id == id) {
            self.message = Some(format!("Tag '{}' already exists", id));
            return Ok(());
        }

        let title = if title_input.is_empty() {
            id.replace('_', " ")
                .split_whitespace()
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            title_input
        };

        let desc = if description.is_empty() {
            format!("Posts about {}.", title.to_lowercase())
        } else {
            description
        };

        let file_path = self.definitions_dir.join(format!("{}.md", id));
        let mut content = format!("---\nid: {}\ntitle: {}\n", id, title);
        if !short.is_empty() {
            content.push_str(&format!("short: \"{}\"\n", short));
        }
        content.push_str("---\n\n");
        content.push_str(&format!("# {}\n\n{}\n", title, desc));

        std::fs::write(&file_path, &content)?;

        let def = TagDefinition {
            id: id.clone(),
            title: title.clone(),
            aliases: vec![],
            short: if short.is_empty() { None } else { Some(short) },
            content,
            file_path: file_path.display().to_string(),
        };
        self.definitions.push(def);
        self.definitions.sort_by(|a, b| a.id.cmp(&b.id));

        self.tag_selections.insert(id.clone());

        self.message = Some(format!("Created tag '{}'", id));
        self.new_tag.clear();
        self.mode = Mode::Normal;
        Ok(())
    }

    fn handle_normal_key(&mut self, key: KeyCode) -> Result<bool> {
        match key {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => {
                if self.selected_tag + 1 < self.definitions.len() {
                    self.selected_tag += 1;
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.selected_tag > 0 {
                    self.selected_tag -= 1;
                }
            }
            KeyCode::Char(' ') => self.toggle_tag(),
            KeyCode::Enter => self.save_current()?,
            KeyCode::Char('s') => self.skip_current(),
            KeyCode::Char('n') => {
                self.mode = Mode::NewTag;
                self.input_field = InputField::Id;
                self.new_tag.clear();
            }
            KeyCode::Char('p') => self.go_prev(),
            _ => {}
        }
        Ok(false)
    }

    fn handle_new_tag_key(&mut self, key: KeyCode) -> Result<bool> {
        match key {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.new_tag.clear();
            }
            KeyCode::Tab => {
                self.input_field = match self.input_field {
                    InputField::Id => InputField::Title,
                    InputField::Title => InputField::Short,
                    InputField::Short => InputField::Description,
                    InputField::Description => InputField::Id,
                };
            }
            KeyCode::BackTab => {
                self.input_field = match self.input_field {
                    InputField::Id => InputField::Description,
                    InputField::Title => InputField::Id,
                    InputField::Short => InputField::Title,
                    InputField::Description => InputField::Short,
                };
            }
            KeyCode::Enter => {
                self.create_tag()?;
            }
            KeyCode::Backspace => {
                self.current_input_mut().pop();
            }
            KeyCode::Char(c) => {
                self.current_input_mut().push(c);
            }
            _ => {}
        }
        Ok(false)
    }

    fn current_input_mut(&mut self) -> &mut String {
        match self.input_field {
            InputField::Id => &mut self.new_tag.id,
            InputField::Title => &mut self.new_tag.title,
            InputField::Short => &mut self.new_tag.short,
            InputField::Description => &mut self.new_tag.description,
        }
    }
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Post panel
    if let Some(post) = app.current_post() {
        let remaining = app.remaining();
        let total = app.posts.len();
        let curated = app.curated_ids.len();
        let post_block = Block::default()
            .title(format!(
                " Post {}/{} | {} curated | {} remaining | @{} ",
                app.current_post + 1,
                total,
                curated,
                remaining,
                post.author
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let text = Text::from(vec![
            Line::from(Span::styled(&post.text, Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(Span::styled(&post.url, Style::default().fg(Color::Blue))),
        ]);

        let paragraph = Paragraph::new(text)
            .block(post_block)
            .wrap(Wrap { trim: false });

        f.render_widget(paragraph, chunks[0]);
    } else {
        let block = Block::default().title(" No posts ").borders(Borders::ALL);
        f.render_widget(block, chunks[0]);
    }

    // Tags panel or new tag form
    match app.mode {
        Mode::Normal => render_tags(f, &mut *app, chunks[1]),
        Mode::NewTag => render_new_tag_form(f, app, chunks[1]),
    }

    // Status bar
    let status_text = if let Some(ref msg) = app.message {
        msg.clone()
    } else {
        match app.mode {
            Mode::Normal => {
                "↑↓/jk: navigate  Space: toggle  Enter: save  s: skip  n: new tag  p: prev  q: quit"
                    .to_string()
            }
            Mode::NewTag => "Tab: next field  Enter: create  Esc: cancel".to_string(),
        }
    };

    let status_style = if app.message.is_some() {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let status = Paragraph::new(Line::from(Span::styled(status_text, status_style))).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(status, chunks[2]);
}

fn render_tags(f: &mut ratatui::Frame, app: &mut App, area: ratatui::layout::Rect) {
    // Adjust scroll offset to keep selected tag visible
    let inner_height = area.height.saturating_sub(2) as usize; // -2 for borders
    if inner_height > 0 {
        if app.selected_tag < app.tag_scroll_offset {
            app.tag_scroll_offset = app.selected_tag;
        } else if app.selected_tag >= app.tag_scroll_offset + inner_height {
            app.tag_scroll_offset = app.selected_tag - inner_height + 1;
        }
    }

    let tag_items: Vec<ListItem> = app
        .definitions
        .iter()
        .enumerate()
        .skip(app.tag_scroll_offset)
        .take(inner_height)
        .map(|(i, def)| {
            let selected = app.tag_selections.contains(&def.id);
            let checkbox = if selected { "[x]" } else { "[ ]" };
            let cursor = if i == app.selected_tag { ">" } else { " " };
            let short = def.short.as_deref().unwrap_or("");
            let text = format!("{} {} {} - {}", cursor, checkbox, def.id, short);
            let style = if i == app.selected_tag {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else if selected {
                Style::default().fg(Color::Green)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(Span::styled(text, style)))
        })
        .collect();

    let selected_tags: Vec<&str> = app.tag_selections.iter().map(|s| s.as_str()).collect();
    let scroll_indicator = if app.definitions.len() > inner_height && inner_height > 0 {
        format!(
            " [{}-{}/{}]",
            app.tag_scroll_offset + 1,
            (app.tag_scroll_offset + inner_height).min(app.definitions.len()),
            app.definitions.len()
        )
    } else {
        String::new()
    };
    let title = if selected_tags.is_empty() {
        format!(" Tags{} (Space: toggle, n: new tag) ", scroll_indicator)
    } else {
        format!(
            " Tags [{}]{} (Space: toggle, n: new tag) ",
            selected_tags.join(", "),
            scroll_indicator
        )
    };

    let tag_block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let list = List::new(tag_items).block(tag_block);
    f.render_widget(list, area);
}

fn render_new_tag_form(f: &mut ratatui::Frame, app: &App, area: ratatui::layout::Rect) {
    let active_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(Color::Gray);
    let cursor = Span::styled("_", Style::default().fg(Color::Yellow));
    let no_cursor = Span::raw("");

    let lines = vec![
        Line::from(vec![
            Span::styled(
                "  ID:    ",
                if matches!(app.input_field, InputField::Id) {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled(&app.new_tag.id, Style::default().fg(Color::White)),
            if matches!(app.input_field, InputField::Id) {
                cursor.clone()
            } else {
                no_cursor.clone()
            },
        ]),
        Line::from(vec![
            Span::styled(
                "  Title: ",
                if matches!(app.input_field, InputField::Title) {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled(&app.new_tag.title, Style::default().fg(Color::White)),
            if matches!(app.input_field, InputField::Title) {
                cursor.clone()
            } else {
                no_cursor.clone()
            },
        ]),
        Line::from(vec![
            Span::styled(
                "  Short: ",
                if matches!(app.input_field, InputField::Short) {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled(&app.new_tag.short, Style::default().fg(Color::White)),
            if matches!(app.input_field, InputField::Short) {
                cursor.clone()
            } else {
                no_cursor.clone()
            },
        ]),
        Line::from(vec![
            Span::styled(
                "  Desc:  ",
                if matches!(app.input_field, InputField::Description) {
                    active_style
                } else {
                    inactive_style
                },
            ),
            Span::styled(&app.new_tag.description, Style::default().fg(Color::White)),
            if matches!(app.input_field, InputField::Description) {
                cursor.clone()
            } else {
                no_cursor.clone()
            },
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Tab: next field  Enter: create  Esc: cancel",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    let form = Paragraph::new(Text::from(lines)).block(
        Block::default()
            .title(" New Tag ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta)),
    );
    f.render_widget(form, area);
}

pub async fn execute(args: CurateArgs, config_path: Option<PathBuf>) -> Result<()> {
    let config = AppConfig::load(config_path.as_deref())?;

    let definitions_dir = args
        .definitions_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(&config.general.definitions_dir));

    let repo = FilesystemDefinitionsRepo::new(&definitions_dir)
        .context("Failed to initialize definitions repository")?;
    let definitions = repo.load().await?;

    let posts = load_posts(&args.input)?;
    if posts.is_empty() {
        println!("No posts found in {}", args.input.display());
        return Ok(());
    }

    let curated_ids = load_curated_ids(&args.output);

    let uncurated = posts
        .iter()
        .filter(|p| !curated_ids.contains(&p.id))
        .count();
    if uncurated == 0 {
        println!("All {} posts already curated!", posts.len());
        return Ok(());
    }

    println!(
        "Loaded {} posts ({} already curated, {} remaining)",
        posts.len(),
        curated_ids.len(),
        uncurated
    );

    let app = run_tui_session(
        posts,
        definitions,
        curated_ids,
        args.output.clone(),
        definitions_dir,
    )?;

    println!(
        "Curated {} posts. Output: {}",
        app.curated_ids.len(),
        args.output.display()
    );

    Ok(())
}

/// Guard that restores terminal state on drop, ensuring cleanup
/// even if setup or the event loop fails partway through.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn run_tui_session(
    posts: Vec<SourcePost>,
    definitions: Vec<TagDefinition>,
    curated_ids: HashSet<String>,
    output_path: PathBuf,
    definitions_dir: PathBuf,
) -> Result<App> {
    enable_raw_mode()?;
    let _guard = TerminalGuard;

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(
        posts,
        definitions,
        curated_ids,
        output_path,
        definitions_dir,
    );
    run_event_loop(&mut terminal, &mut app)?;

    // _guard drops here, restoring terminal state
    Ok(app)
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| ui(f, &mut *app))?;

        if app.should_quit {
            break;
        }

        if let Event::Key(key) = event::read()? {
            app.message = None;

            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }

            let quit = match app.mode {
                Mode::Normal => app.handle_normal_key(key.code)?,
                Mode::NewTag => app.handle_new_tag_key(key.code)?,
            };

            if quit {
                break;
            }
        }
    }
    Ok(())
}

fn load_posts(path: &PathBuf) -> Result<Vec<SourcePost>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    let mut posts = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(post) = serde_json::from_str::<SourcePost>(line) {
            posts.push(post);
        }
    }
    Ok(posts)
}

fn load_curated_ids(path: &PathBuf) -> HashSet<String> {
    let mut ids = HashSet::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return ids,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(curated) = serde_json::from_str::<CuratedPost>(line) {
            ids.insert(curated.post_id);
        }
    }
    ids
}
