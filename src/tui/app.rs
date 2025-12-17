use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use diffsoup::diff::CommitDiff;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};
use std::{io, sync::mpsc::Receiver, thread::JoinHandle, time::Duration};

use crate::tui::{
    UiSender,
    state::{AppScreen, ScrollDirection, ScrollEvent, UiEvent},
};

pub fn spawn_ui_thread(
    action_tx: UiSender,
    view_rx: Receiver<AppScreen>,
) -> JoinHandle<anyhow::Result<()>> {
    let mut screen = AppScreen::Loading;
    std::thread::spawn(move || {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let _ = terminal.size().inspect(|size| {
            let _ = action_tx.send(UiEvent::SizeChange((size.width, size.height)));
        });

        while !matches!(screen, AppScreen::Exit) {
            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Resize(width, height) => {
                        action_tx.send(UiEvent::SizeChange((width, height)))?;
                    }
                    Event::Key(event) => {
                        if let Some(action) = handle_event(&event, &screen) {
                            action_tx.send(action)?;
                        }
                    }
                    _ => {}
                }
            }

            if let Ok(view) = view_rx.try_recv() {
                terminal.draw(|f| draw(&view, f))?;
                screen = view;
            }
        }

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;
        Ok(())
    })
}

fn handle_event(event: &KeyEvent, screen: &AppScreen) -> Option<UiEvent> {
    if event.kind != KeyEventKind::Press {
        return None;
    }

    // Global bindings (work in all screens)
    match (event.code, event.modifiers) {
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            return Some(UiEvent::Exit);
        }
        _ => {}
    }

    // Screen-specific bindings
    match screen {
        AppScreen::List(list_view) => handle_list_keys(event, list_view),
        AppScreen::DiffView(_) => handle_diff_keys(event),
        _ => None,
    }
}

fn handle_list_keys(event: &KeyEvent, list_view: &crate::tui::state::ListView) -> Option<UiEvent> {
    match (event.code, event.modifiers) {
        (KeyCode::Char('q'), _) => Some(UiEvent::Exit),
        (KeyCode::Down | KeyCode::Char('j'), _) => {
            Some(UiEvent::Scroll(ScrollEvent::Single(ScrollDirection::Down)))
        }
        (KeyCode::Up | KeyCode::Char('k'), _) => {
            Some(UiEvent::Scroll(ScrollEvent::Single(ScrollDirection::Up)))
        }
        (KeyCode::Enter | KeyCode::Char('l'), _) => {
            list_view.list_state.selected().map(UiEvent::EnterDiff)
        }
        (KeyCode::Char('h'), _) => Some(UiEvent::ToggleUnchanged),
        (KeyCode::Char('['), _) => {
            if list_view.base_index > 0 {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index - 1,
                    list_view.comparison_index,
                )))
            } else {
                None
            }
        }
        (KeyCode::Char(']'), _) => {
            if list_view.base_index < list_view.comparison_index {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index + 1,
                    list_view.comparison_index,
                )))
            } else {
                None
            }
        }
        (KeyCode::Char('{'), _) => {
            if list_view.comparison_index > list_view.base_index {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index,
                    list_view.comparison_index - 1,
                )))
            } else {
                None
            }
        }
        (KeyCode::Char('}'), _) => {
            if list_view.comparison_index < list_view.total_commits - 1 {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index,
                    list_view.comparison_index + 1,
                )))
            } else {
                None
            }
        }
        (KeyCode::Char('<'), _) => {
            if list_view.base_index > 0 && list_view.comparison_index > 0 {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index - 1,
                    list_view.comparison_index - 1,
                )))
            } else {
                None
            }
        }
        (KeyCode::Char('>'), _) => {
            let max_index = list_view.total_commits - 1;
            if list_view.base_index < max_index && list_view.comparison_index < max_index {
                Some(UiEvent::PatchsetChange((
                    list_view.base_index + 1,
                    list_view.comparison_index + 1,
                )))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn handle_diff_keys(event: &KeyEvent) -> Option<UiEvent> {
    match (event.code, event.modifiers) {
        (KeyCode::Up | KeyCode::Char('k'), KeyModifiers::NONE) => {
            Some(UiEvent::Scroll(ScrollEvent::Single(ScrollDirection::Up)))
        }
        (KeyCode::Down | KeyCode::Char('j'), KeyModifiers::NONE) => {
            Some(UiEvent::Scroll(ScrollEvent::Single(ScrollDirection::Down)))
        }
        (KeyCode::Char('d'), KeyModifiers::CONTROL) => Some(UiEvent::Scroll(
            ScrollEvent::HalfPage(ScrollDirection::Down),
        )),
        (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
            Some(UiEvent::Scroll(ScrollEvent::HalfPage(ScrollDirection::Up)))
        }
        (KeyCode::Char('f'), KeyModifiers::CONTROL) | (KeyCode::PageDown, _) => Some(
            UiEvent::Scroll(ScrollEvent::FullPage(ScrollDirection::Down)),
        ),
        (KeyCode::Char('b'), KeyModifiers::CONTROL) | (KeyCode::PageUp, _) => {
            Some(UiEvent::Scroll(ScrollEvent::FullPage(ScrollDirection::Up)))
        }
        (KeyCode::Char('g'), _) => Some(UiEvent::Scroll(ScrollEvent::Top)),
        (KeyCode::Char('G'), _) => Some(UiEvent::Scroll(ScrollEvent::Bottom)),
        (KeyCode::Backspace | KeyCode::Left | KeyCode::Char('q'), KeyModifiers::NONE) => {
            Some(UiEvent::BackToList)
        }
        (KeyCode::Char('y'), KeyModifiers::NONE) => Some(UiEvent::CopyToClipboard),
        _ => None,
    }
}

fn draw(screen: &AppScreen, f: &mut ratatui::Frame) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    // Render header
    let header_text = match screen {
        AppScreen::Loading => "diffsoup - Loading...".to_string(),
        AppScreen::Exit => "diffsoup - Exiting...".to_string(),
        AppScreen::Error(_) => "diffsoup - Error".to_string(),
        AppScreen::List(list_view) => {
            let total = list_view.total_commits;
            format!(
                "diffsoup - Patchset [{}/{}] {} → [{}/{}] {}",
                list_view.base_index + 1,
                total,
                list_view.base_name,
                list_view.comparison_index + 1,
                total,
                list_view.comparison_name
            )
        }
        AppScreen::DiffView(_) => "diffsoup - Interdiff View".to_string(),
    };

    let header = Paragraph::new(header_text)
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    // Render main content
    match screen {
        AppScreen::Loading => {
            render_message(f, chunks[1], "Loading...");
        }
        AppScreen::Exit => {}
        AppScreen::Error(Some(msg)) => {
            render_message(f, chunks[1], msg);
        }
        AppScreen::Error(None) => {
            render_message(f, chunks[1], "An error occurred");
        }
        AppScreen::List(list_view) => {
            render_list(f, chunks[1], list_view);
        }
        AppScreen::DiffView(diff_view) => {
            render_interdiff(f, chunks[1], diff_view);
        }
    }

    // Render footer
    let footer_text = match screen {
        AppScreen::Loading | AppScreen::Exit | AppScreen::Error(_) => "".to_string(),
        AppScreen::List(list_view) => {
            let hide_text = if list_view.show_unchanged {
                "hide"
            } else {
                "show"
            };
            format!(
                "q: Quit | ↑↓/jk: Navigate | Enter: View | h: {} unchanged | []: Base | {{}}: Comp | <>: Both",
                hide_text
            )
        }
        AppScreen::DiffView(_) => "q: Back | ↑↓: Scroll | y: Copy diff to clipboard".to_string(),
    };

    let footer = Paragraph::new(footer_text)
        .style(Style::default().fg(Color::Gray))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(footer, chunks[2]);
}

fn render_message(f: &mut ratatui::Frame, area: ratatui::layout::Rect, msg: &str) {
    let lines: Vec<Line> = msg.lines().map(Line::from).collect();
    let block = Block::default().borders(Borders::ALL);
    let content = Paragraph::new(lines).block(block);
    f.render_widget(content, area);
}

fn render_list(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    list_view: &crate::tui::state::ListView,
) {
    let visible_commits = list_view.get_visible_commits();

    let title = format!(
        "Commit Comparison: {} → {} ({}/{} commits{})",
        list_view.base_name,
        list_view.comparison_name,
        visible_commits.len(),
        list_view.commits.len(),
        if list_view.show_unchanged {
            ""
        } else {
            ", changed only"
        }
    );

    let items: Vec<ListItem> = visible_commits
        .iter()
        .map(|commit| format_commit_item(commit))
        .collect();

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().reversed().add_modifier(Modifier::BOLD));

    // Clone ListState for rendering (cheap, just a couple usizes)
    let mut list_state = list_view.list_state.clone();
    f.render_stateful_widget(list, area, &mut list_state);
}

fn format_commit_item(commit: &CommitDiff) -> ListItem<'_> {
    let has_changes = commit.has_changes();

    let (status_icon, base_style) = match (&commit.from, &commit.to) {
        (None, Some(_)) => ("+ ", Style::default().fg(Color::Green)),
        (Some(_), None) => ("- ", Style::default().fg(Color::Red)),
        (Some(from), Some(to)) if from.message != to.message => {
            ("✎ ", Style::default().fg(Color::Cyan))
        }
        (Some(from), Some(to)) if from.sha != to.sha && has_changes => {
            ("~ ", Style::default().fg(Color::Yellow))
        }
        _ => ("  ", Style::default().fg(Color::DarkGray)),
    };

    let style = if !has_changes {
        Style::default().fg(Color::DarkGray)
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

    let line = Line::from(vec![
        Span::styled(status_icon, style),
        Span::styled(format!("{:<16} ", sha_info), style),
        Span::styled(message, style),
        Span::styled(stats_text, Style::default().fg(Color::DarkGray)),
    ]);

    ListItem::new(line).style(style)
}

fn render_interdiff(
    f: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    diff_view: &crate::tui::state::DiffView,
) {
    let lines: Vec<Line> = diff_view
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

    let length: u16 = lines.len().try_into().unwrap_or(u16::MAX);
    let scroll = diff_view.scroll.min(length);

    let block = Block::default()
        .title_top(format!("Interdiff View: {}", diff_view.commit))
        .title_bottom(format!("{} / {}", scroll, length))
        .borders(Borders::ALL);

    let content = Paragraph::new(lines).block(block).scroll((scroll, 0));

    f.render_widget(content, area);
}
