use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diffsoup::{
    diff::{BranchDiff, CommitDiff, calculate_branch_diff, get_commit, render_interdiff},
    error::{CustomError, Result},
    trees::DiffTree,
};
use error_stack::ResultExt;
use jj_lib::{
    ref_name::RefNameBuf,
    repo::{ReadonlyRepo, Repo},
    workspace::Workspace,
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Size},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use std::{io, sync::Arc, time::Duration};

pub struct App {
    should_quit: bool,
    current_screen: Screen,
    workspace: Workspace,
    repo: Arc<ReadonlyRepo>,
    state: AppState,
}

#[derive(Debug)]
pub struct DiffView {
    commit: String,
    diff: String,
    rendered_width: u16,
}

#[derive(Debug)]
enum Screen {
    Empty,
    CommitList(BranchDiff),
    InterdiffView(DiffView),
}

#[derive(Debug, Default)]
pub struct AppState {
    commit_history: Vec<RefNameBuf>,
    base_branch: usize,
    comparison_branch: usize,
    displayed_base: Option<usize>,
    displayed_comparison: Option<usize>,
    selected_commit_index: usize,
    list_state: ListState,
    show_unchanged: bool,
    cache: Option<BranchDiff>,
    interdiff_scroll: u16,
}

impl App {
    pub fn new(workspace: Workspace) -> Result<Self> {
        let repo = workspace
            .repo_loader()
            .load_at_head()
            .change_context(CustomError::RepoError)?;
        Ok(Self {
            should_quit: false,
            current_screen: Screen::Empty,
            workspace,
            repo,
            state: AppState {
                commit_history: Vec::new(),
                base_branch: 0,
                comparison_branch: 1,
                displayed_base: None,
                displayed_comparison: None,
                selected_commit_index: 0,
                list_state: ListState::default(),
                show_unchanged: false,
                cache: None,
                interdiff_scroll: 0,
            },
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    pub fn set_commit_history(&mut self, history: Vec<RefNameBuf>) {
        self.state.commit_history = history;
    }

    pub fn get_overview(&mut self) {
        if let Some(diff) = self.state.cache.take() {
            self.current_screen = Screen::CommitList(diff);
            return;
        }
        let (Some(from), Some(to)) = (
            self.state.commit_history.get(self.state.base_branch),
            self.state.commit_history.get(self.state.comparison_branch),
        ) else {
            return;
        };
        self.state.selected_commit_index = 0;
        self.state.list_state.select(Some(0));
        self.current_screen = match calculate_branch_diff(
            from.as_str(),
            to.as_str(),
            &self.workspace,
            self.repo.as_ref(),
        ) {
            Ok(diff) => {
                self.state.displayed_base = Some(self.state.base_branch);
                self.state.displayed_comparison = Some(self.state.comparison_branch);
                Screen::CommitList(diff)
            }
            _ => Screen::Empty,
        };
    }

    fn event_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> io::Result<()> {
        self.get_overview();
        let mut last_size = terminal.size()?;
        let mut needs_redraw = true;
        while !self.should_quit {
            if event::poll(Duration::from_millis(100))? {
                match event::read()? {
                    Event::Resize(width, height) => {
                        last_size = Size { width, height };
                        needs_redraw = true;
                    }
                    Event::Key(event) if event.kind == KeyEventKind::Press => {
                        self.handle_key(event, terminal);
                        needs_redraw = true;
                    }
                    _ => {}
                }
            }

            if matches!(self.current_screen, Screen::CommitList(_))
                && (self.state.displayed_base != Some(self.state.base_branch)
                    || self.state.displayed_comparison != Some(self.state.comparison_branch))
            {
                self.state.cache = None;
                self.get_overview();
                needs_redraw = true;
            }

            if let (Screen::InterdiffView(view), Some(branch_diff)) =
                (&self.current_screen, &self.state.cache)
                && view.rendered_width != last_size.width
            {
                let Some((diff, title)) = App::get_interdiff(
                    self.get_selected_commit(branch_diff),
                    &self.workspace,
                    self.repo.as_ref(),
                    last_size.width,
                ) else {
                    continue;
                };
                self.current_screen = Screen::InterdiffView(DiffView {
                    commit: title,
                    diff,
                    rendered_width: last_size.width,
                });
            }

            if needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                needs_redraw = false;
            }
        }
        Ok(())
    }

    fn get_selected_commit<'a>(&self, branch_diff: &'a BranchDiff) -> &'a CommitDiff {
        if self.state.show_unchanged {
            &branch_diff.commits[self.state.selected_commit_index]
        } else {
            let commits = branch_diff
                .commits
                .iter()
                .filter(|c| c.has_changes())
                .collect::<Vec<_>>();
            commits[self.state.selected_commit_index]
        }
    }

    fn get_interdiff(
        commit_diff: &CommitDiff,
        workspace: &Workspace,
        repo: &dyn Repo,
        render_width: u16,
    ) -> Option<(String, String)> {
        let from_commit = if let Some(from) = &commit_diff.from {
            get_commit(&from.sha, workspace, repo).ok()
        } else {
            None
        };
        let to_commit = if let Some(to) = &commit_diff.to {
            get_commit(&to.sha, workspace, repo).ok()
        } else {
            None
        };
        let trees = DiffTree::from(from_commit.as_ref(), to_commit.as_ref());
        if let Some(trees) = trees {
            return Some((
                render_interdiff(&trees, workspace, repo, render_width)
                    .unwrap_or_else(|e| format!("{}", e)),
                format!("{trees}"),
            ));
        };
        None
    }

    fn handle_key(&mut self, key: KeyEvent, terminal: &Terminal<CrosstermBackend<io::Stdout>>) {
        match &self.current_screen {
            Screen::CommitList(_) => self.handle_list_keys(key, terminal),
            Screen::InterdiffView(_) => self.handle_interdiff_keys(key, terminal),
            Screen::Empty => {}
        }
        // Global bindings, not specific to any screen
        match (key.code, key.modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.should_quit = true,
            (_, _) => {}
        }
    }

    fn handle_list_keys(
        &mut self,
        key: KeyEvent,
        terminal: &Terminal<CrosstermBackend<io::Stdout>>,
    ) {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => self.should_quit = true,
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                if self.state.selected_commit_index > 0 {
                    self.state.selected_commit_index -= 1;
                    self.state
                        .list_state
                        .select(Some(self.state.selected_commit_index));
                }
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                if let Screen::CommitList(ref diff) = self.current_screen {
                    let visible_commits = self.get_visible_commits(diff);
                    if self.state.selected_commit_index < visible_commits.len().saturating_sub(1) {
                        self.state.selected_commit_index += 1;
                        self.state
                            .list_state
                            .select(Some(self.state.selected_commit_index));
                    }
                }
            }
            (KeyCode::Enter | KeyCode::Char('l'), _) => {
                if let Screen::CommitList(branch_diff) =
                    std::mem::replace(&mut self.current_screen, Screen::Empty)
                {
                    let Ok(size) = terminal.size() else {
                        return;
                    };
                    if let Some((diff, title)) = App::get_interdiff(
                        self.get_selected_commit(&branch_diff),
                        &self.workspace,
                        self.repo.as_ref(),
                        size.width,
                    ) {
                        self.current_screen = Screen::InterdiffView(DiffView {
                            commit: title,
                            diff,
                            rendered_width: size.width,
                        });
                        self.state.cache = Some(branch_diff);
                    } else {
                        self.current_screen = Screen::CommitList(branch_diff);
                    }
                }
            }
            (KeyCode::Char('h'), _) => {
                self.state.show_unchanged = !self.state.show_unchanged;
                self.state.selected_commit_index = 0;
                self.state.list_state.select(Some(0));
            }
            (_, _) => {}
        }
    }

    fn handle_interdiff_keys(
        &mut self,
        key: KeyEvent,
        terminal: &Terminal<CrosstermBackend<io::Stdout>>,
    ) {
        match (key.code, key.modifiers) {
            (KeyCode::Up | KeyCode::Char('k'), _) => {
                self.state.interdiff_scroll = self.state.interdiff_scroll.saturating_sub(1);
            }
            (KeyCode::Down | KeyCode::Char('j'), _) => {
                self.state.interdiff_scroll = self.state.interdiff_scroll.saturating_add(1);
            }
            (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.state.interdiff_scroll = self
                    .state
                    .interdiff_scroll
                    .saturating_add(terminal.size().unwrap_or_default().height / 2);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.state.interdiff_scroll = self
                    .state
                    .interdiff_scroll
                    .saturating_sub(terminal.size().unwrap_or_default().height / 2);
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('f'), KeyModifiers::CONTROL) => {
                self.state.interdiff_scroll = self
                    .state
                    .interdiff_scroll
                    .saturating_add(terminal.size().unwrap_or_default().height);
            }
            (KeyCode::PageUp, _) | (KeyCode::Char('b'), KeyModifiers::CONTROL) => {
                self.state.interdiff_scroll = self
                    .state
                    .interdiff_scroll
                    .saturating_sub(terminal.size().unwrap_or_default().height);
            }
            (KeyCode::Backspace | KeyCode::Left | KeyCode::Char('q'), _) => {
                self.state.interdiff_scroll = 0;
                self.get_overview();
            }
            _ => {}
        }
    }

    fn get_visible_commits<'a>(&self, diff: &'a BranchDiff) -> Vec<&'a diffsoup::diff::CommitDiff> {
        if self.state.show_unchanged {
            diff.commits.iter().collect()
        } else {
            diff.commits.iter().filter(|c| c.has_changes()).collect()
        }
    }

    fn ui(&mut self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        let header_text = match &self.current_screen {
            Screen::Empty => "diffsoup - Branch Comparison Tool".to_string(),
            Screen::CommitList(_) => {
                format!(
                    "diffsoup - {} vs {}",
                    self.state
                        .commit_history
                        .get(self.state.base_branch)
                        .as_ref()
                        .map(|b| b.as_str())
                        .unwrap_or("?"),
                    self.state
                        .commit_history
                        .get(self.state.comparison_branch)
                        .as_ref()
                        .map(|b| b.as_str())
                        .unwrap_or("?")
                )
            }
            Screen::InterdiffView(_) => "diffsoup - Interdiff View".to_string(),
        };
        let header = Paragraph::new(header_text)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(header, chunks[0]);

        match &self.current_screen {
            Screen::Empty => {}
            Screen::CommitList(branch_diff) => {
                let visible_commits = self.get_visible_commits(branch_diff);
                let title = self.get_commit_list_title(branch_diff);
                let items: Vec<ListItem> = visible_commits
                    .iter()
                    .enumerate()
                    .map(|(idx, commit)| {
                        self.format_commit_item(commit, idx == self.state.selected_commit_index)
                    })
                    .collect();

                let block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan));

                let list = List::new(items)
                    .block(block)
                    .highlight_style(Style::default().reversed().add_modifier(Modifier::BOLD));

                f.render_stateful_widget(list, chunks[1], &mut self.state.list_state);
            }
            Screen::InterdiffView(diff) => {
                self.state.interdiff_scroll = self.render_interdiff(f, chunks[1], diff)
            }
        }

        let footer_text = match &self.current_screen {
            Screen::Empty => "".to_string(),
            Screen::CommitList(_) => {
                let hide_text = if self.state.show_unchanged {
                    "hide"
                } else {
                    "show"
                };
                format!(
                    "q: Quit | ↑↓/jk: Navigate | Enter: View interdiff | h: {} unchanged",
                    hide_text
                )
            }
            Screen::InterdiffView(_) => "q: Back | ↑↓: Scroll".to_string(),
        };
        let footer = Paragraph::new(footer_text)
            .style(Style::default().fg(Color::Gray))
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(footer, chunks[2]);
    }

    fn get_commit_list_title(&self, diff: &BranchDiff) -> String {
        let visible_commits = self.get_visible_commits(diff);
        let from_branch = self
            .state
            .commit_history
            .get(self.state.base_branch)
            .as_ref()
            .map(|b| b.as_str())
            .unwrap_or("<not set>");
        let to_branch = self
            .state
            .commit_history
            .get(self.state.comparison_branch)
            .as_ref()
            .map(|b| b.as_str())
            .unwrap_or("<not set>");

        format!(
            "Commit Comparison: {} → {} ({}/{} commits{})",
            from_branch,
            to_branch,
            visible_commits.len(),
            diff.commits.len(),
            if self.state.show_unchanged {
                ""
            } else {
                ", changed only"
            }
        )
    }

    fn format_commit_item<'a>(
        &self,
        commit: &'a diffsoup::diff::CommitDiff,
        is_selected: bool,
    ) -> ListItem<'a> {
        let has_changes = commit.has_changes();

        let (status_icon, base_style) = match (&commit.from, &commit.to) {
            (None, Some(_)) => ("+ ", Style::default().fg(Color::Green)),
            (Some(_), None) => ("- ", Style::default().fg(Color::Red)),
            (Some(from), Some(to)) if from.message != to.message => {
                ("✎ ", Style::default().fg(Color::Cyan))
            }
            (Some(_), Some(_)) if has_changes => ("~ ", Style::default().fg(Color::Yellow)),
            _ => ("  ", Style::default().fg(Color::DarkGray)),
        };

        let style = if !has_changes && !self.state.show_unchanged {
            Style::default().fg(Color::DarkGray)
        } else if is_selected {
            base_style
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::REVERSED)
        } else {
            base_style
        };

        let message = commit
            .to
            .as_ref()
            .or(commit.from.as_ref())
            .and_then(|m| m.message.split_once('\n'))
            .map(|(subject, _)| subject)
            .unwrap_or("<no message>");

        let sha_info = match (&commit.from, &commit.to) {
            (Some(from), Some(to)) if from.sha != to.sha => {
                format!("{} → {}", &from.sha[..8], &to.sha[..8])
            }
            (Some(c), None) => format!("{} (removed)", &c.sha[..8]),
            (None, Some(c)) => format!("{} (new)", &c.sha[..8]),
            (Some(c), Some(_)) => c.sha[..8].to_string(),
            _ => "????????".to_string(),
        };

        let stats_text = if commit.stats.changed_files > 0 {
            format!(
                " [±{} files, +{}, -{}]",
                commit.stats.changed_files, commit.stats.additions, commit.stats.removals
            )
        } else {
            String::new()
        };

        let cursor = if is_selected { "▶ " } else { "  " };

        let line = Line::from(vec![
            Span::styled(cursor, style),
            Span::styled(status_icon, style),
            Span::styled(format!("{:<16} ", sha_info), style),
            Span::styled(message, style),
            Span::styled(stats_text, Style::default().fg(Color::DarkGray)),
        ]);

        ListItem::new(line).style(style)
    }

    fn render_interdiff(
        &self,
        f: &mut ratatui::Frame,
        area: ratatui::layout::Rect,
        diff: &DiffView,
    ) -> u16 {
        let lines: Vec<Line> = diff
            .diff
            .lines()
            .map(|line| {
                let style = if line.starts_with('+') && !line.starts_with("+++") {
                    Style::default().fg(Color::Green)
                } else if line.starts_with('-') && !line.starts_with("---") {
                    Style::default().fg(Color::Red)
                } else if line.starts_with("@@") {
                    Style::default().fg(Color::Cyan)
                } else if line.starts_with("diff") || line.starts_with("index") {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(line.to_string(), style))
            })
            .collect();
        let length: u16 = lines.len().try_into().unwrap_or_default();
        let scroll = if self.state.interdiff_scroll > length {
            length
        } else {
            self.state.interdiff_scroll
        };

        let block = Block::default()
            .title_top(format!("Interdiff View: {}", diff.commit))
            .title_bottom(format!("{} / {}", scroll, length))
            .borders(Borders::ALL);

        let content = Paragraph::new(lines).block(block).scroll((scroll, 0));

        f.render_widget(content, area);

        scroll
    }
}
