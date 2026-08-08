use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, JunkState, Panel};

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(0)])
        .split(root[0]);

    draw_sidebar(f, app, body[0]);
    match app.panel {
        Panel::Junk => draw_junk(f, app, body[1]),
        Panel::Memory => draw_memory(f, app, body[1]),
    }
    draw_status(f, app, root[1]);
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Sidebar;
    let items = [("Junk Cleanup", Panel::Junk), ("Memory", Panel::Memory)]
        .into_iter()
        .map(|(label, panel)| {
            let style = if panel == app.panel {
                Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!(" {label}")).style(style)
        })
        .collect::<Vec<_>>();

    let border_style = if focused { Style::default().fg(Color::Cyan) } else { Style::default() };
    let list = List::new(items).block(Block::default().borders(Borders::ALL).title("macsweep").border_style(border_style));
    f.render_widget(list, area);
}

fn draw_junk(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Junk Cleanup");

    match &app.junk {
        JunkState::Blank => {
            let p = Paragraph::new("Press [s] to scan for junk files.").block(block);
            f.render_widget(p, area);
        }
        JunkState::Scanning(_) => {
            let p = Paragraph::new("Scanning...").block(block);
            f.render_widget(p, area);
        }
        JunkState::Review { entries, cursor, .. } => {
            let inner = block.inner(area);
            f.render_widget(block, area);
            let lines: Vec<ListItem> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let mark = if e.selected { "☑" } else { "☐" };
                    let skip_note =
                        if e.skipped_running > 0 { format!("  ({} in use, skipped)", e.skipped_running) } else { String::new() };
                    let text = format!(
                        "{mark} {:<22} {:>10}  {}{}",
                        e.category.label(),
                        human_size(e.total_size),
                        e.file_count_label(),
                        skip_note
                    );
                    let style = if i == *cursor {
                        Style::default().add_modifier(Modifier::REVERSED)
                    } else {
                        Style::default()
                    };
                    ListItem::new(text).style(style)
                })
                .collect();
            f.render_widget(List::new(lines), inner);
        }
        JunkState::Cleaning { current, done_bytes, total_bytes, .. } => {
            let inner = block.inner(area);
            f.render_widget(block, area);
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Min(0)])
                .split(inner);
            let ratio = if *total_bytes == 0 { 0.0 } else { (*done_bytes as f64 / *total_bytes as f64).min(1.0) };
            let gauge = Gauge::default()
                .block(Block::default().title("Cleaning"))
                .gauge_style(Style::default().fg(Color::Cyan))
                .ratio(ratio)
                .label(format!("{} / {}", human_size(*done_bytes), human_size(*total_bytes)));
            f.render_widget(gauge, rows[0]);
            f.render_widget(Paragraph::new(current.as_str()), rows[1]);
        }
        JunkState::Summary { freed, per_category } => {
            let inner = block.inner(area);
            f.render_widget(block, area);
            let mut lines = vec![Line::from(Span::styled(
                format!("Freed {}", human_size(*freed)),
                Style::default().add_modifier(Modifier::BOLD),
            ))];
            lines.push(Line::from(""));
            for (label, size) in per_category {
                lines.push(Line::from(format!("  {label:<22} {}", human_size(*size))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from("Press any key to rescan."));
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

fn draw_memory(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title("Memory");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let stats = &app.mem.stats;
    let used_gauge = Gauge::default()
        .block(Block::default().title(format!(
            "Used ({} / {})",
            human_size(stats.used_bytes),
            human_size(stats.total_bytes)
        )))
        .gauge_style(Style::default().fg(Color::Yellow))
        .ratio((stats.pressure_pct() / 100.0).clamp(0.0, 1.0));
    f.render_widget(used_gauge, rows[0]);

    let swap_gauge = Gauge::default()
        .block(Block::default().title(format!(
            "Swap ({} / {})",
            human_size(stats.swap_used_bytes),
            human_size(stats.swap_total_bytes)
        )))
        .gauge_style(Style::default().fg(Color::Magenta))
        .ratio((stats.swap_pct() / 100.0).clamp(0.0, 1.0));
    f.render_widget(swap_gauge, rows[1]);

    f.render_widget(Paragraph::new("[p] Free Up RAM (sudo purge)"), rows[2]);

    let avail_line = format!("Available: {}", human_size(stats.available_bytes));
    let line = app.mem.status.as_deref().map_or(avail_line.clone(), |s| format!("{avail_line}  ·  {s}"));
    f.render_widget(Paragraph::new(line), rows[3]);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    f.render_widget(Paragraph::new(app.status.as_str()), area);
}
