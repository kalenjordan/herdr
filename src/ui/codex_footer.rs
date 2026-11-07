//! Presentation-only replacement of Codex's shortcut hint row. The PTY is untouched.
use ratatui::{
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::text::display_width_u16;
use crate::{
    app::AppState,
    layout::PaneInfo,
    terminal::{TerminalRuntime, TerminalRuntimeRegistry},
};

// Require both the bottom hint and nearby composer. Never search arbitrary reply text.
fn is_hint_footer(text: &str) -> bool {
    let lines: Vec<_> = text.lines().collect();
    let Some(last) = lines.last() else {
        return false;
    };
    last.contains("? for shortcuts")
        && lines
            .iter()
            .rev()
            .skip(1)
            .take(6)
            .any(|line| line.trim_start().starts_with('›'))
}

fn footer_rect(app: &AppState, info: &PaneInfo, rt: &TerminalRuntime) -> Option<Rect> {
    let ws_idx = app.active?;
    let terminal_id = app.workspaces.get(ws_idx)?.terminal_id(info.id)?;
    let terminal = app.terminals.get(terminal_id)?;
    if terminal.effective_known_agent() != Some(crate::detect::Agent::Codex)
        || super::panes::pane_is_scrolled_back(rt)
        || info.inner_rect.width == 0
        || info.inner_rect.height < 3
    {
        return None;
    }
    let text = rt.visible_text();
    if !is_hint_footer(&text) {
        return None;
    }
    let row = u16::try_from(text.lines().count().checked_sub(1)?).ok()?;
    if row >= info.inner_rect.height {
        return None;
    }
    Some(Rect::new(
        info.inner_rect.x,
        info.inner_rect.y + row,
        info.inner_rect.width,
        1,
    ))
}

fn link_targets(links: &[crate::codex_usage::ReplyLink], rect: Rect) -> Vec<(Rect, String, &str)> {
    let mut x = rect.x.saturating_add(2);
    let mut targets = Vec::new();
    for link in links.iter().take(2) {
        if x >= rect.right() {
            break;
        }
        let label = reply_url_label(link);
        let width = display_width_u16(&label).min(rect.right().saturating_sub(x));
        let target = Rect::new(x, rect.y, width, 1);
        x = x.saturating_add(width).saturating_add(2);
        targets.push((target, label, link.url.as_str()));
    }
    targets
}

pub(super) fn render(app: &AppState, frame: &mut Frame, info: &PaneInfo, rt: &TerminalRuntime) {
    let Some(rect) = footer_rect(app, info, rt) else {
        return;
    };
    // Retain the terminal's background, including light themes, while erasing hints.
    let bg = frame.buffer_mut()[(rect.x, rect.y)].bg;
    for x in rect.x..rect.right() {
        frame.buffer_mut()[(x, rect.y)].reset();
        frame.buffer_mut()[(x, rect.y)].set_bg(bg);
    }
    if !info.is_focused {
        return;
    }
    for (target, label, _) in link_targets(&app.recent_reply_links, rect) {
        let hovered = app.mouse_capture
            && app
                .mouse_position
                .is_some_and(|position| target.contains(position));
        let color = if hovered {
            app.palette.accent
        } else {
            reply_link_color(&app.palette)
        };
        let style = Style::default()
            .fg(color)
            .bg(bg)
            .add_modifier(Modifier::UNDERLINED);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(label, style))),
            target,
        );
    }
}

pub(crate) fn codex_footer_contains(
    app: &AppState,
    runtimes: &TerminalRuntimeRegistry,
    position: Position,
) -> bool {
    let Some(ws_idx) = app.active else {
        return false;
    };
    app.view.pane_infos.iter().any(|info| {
        app.runtime_for_pane_in_workspace(runtimes, ws_idx, info.id)
            .and_then(|rt| footer_rect(app, info, rt))
            .is_some_and(|rect| rect.contains(position))
    })
}

pub(crate) fn recent_reply_link_at<'a>(
    app: &'a AppState,
    runtimes: &TerminalRuntimeRegistry,
    position: Position,
) -> Option<&'a str> {
    let ws_idx = app.active?;
    let info = app.view.pane_infos.iter().find(|info| info.is_focused)?;
    let rt = app.runtime_for_pane_in_workspace(runtimes, ws_idx, info.id)?;
    let rect = footer_rect(app, info, rt)?;
    link_targets(&app.recent_reply_links, rect)
        .into_iter()
        .find_map(|(target, _, url)| target.contains(position).then_some(url))
}
fn reply_link_color(palette: &crate::app::state::Palette) -> Color {
    match palette.overlay1 {
        Color::Rgb(r, g, b) => Color::Rgb(
            r.saturating_add(12),
            g.saturating_add(12),
            b.saturating_add(12),
        ),
        _ => palette.overlay1,
    }
}

fn reply_url_label(link: &crate::codex_usage::ReplyLink) -> String {
    let url = link.url.as_str();
    let text = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    link.label.as_deref().unwrap_or(text).to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn replaces_hints_without_changing_terminal_and_updates_click_targets() {
        use crate::{
            detect::{Agent, AgentState},
            workspace::Workspace,
        };
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = AppState::test_new();
        app.active = Some(0);
        app.workspaces = vec![Workspace::test_new("test")];
        app.ensure_test_terminals();
        let pane_id = app.workspaces[0].focused_pane_id().unwrap();
        let terminal_id = app.workspaces[0].terminal_id(pane_id).unwrap().clone();
        app.terminals
            .get_mut(&terminal_id)
            .unwrap()
            .set_detected_state(Some(Agent::Codex), AgentState::Idle);
        let rt = TerminalRuntime::test_with_screen_bytes(80, 6, b"reply\r\n\r\n> unused\r\n");
        rt.test_process_pty_bytes(
            "\x1b[4;1H› Ask Codex\x1b[5;1Hmodel · thread\x1b[6;1H  ← for agents · ? for shortcuts"
                .as_bytes(),
        );
        let original = rt.visible_text();
        let info = PaneInfo {
            id: pane_id,
            rect: Rect::new(0, 0, 80, 6),
            inner_rect: Rect::new(0, 0, 80, 6),
            scrollbar_rect: None,
            borders: ratatui::widgets::Borders::NONE,
            is_focused: true,
        };
        app.view.pane_infos = vec![info.clone()];
        let mut runtimes = TerminalRuntimeRegistry::default();
        runtimes.insert(terminal_id, rt);
        let mut terminal = Terminal::new(TestBackend::new(80, 6)).unwrap();
        let draw = |app: &AppState, terminal: &mut Terminal<TestBackend>| {
            let rt = app
                .runtime_for_pane_in_workspace(&runtimes, 0, pane_id)
                .unwrap();
            terminal
                .draw(|frame| {
                    rt.render(frame, info.inner_rect, false);
                    render(app, frame, &info, rt);
                })
                .unwrap();
        };
        draw(&app, &mut terminal);
        assert!((0..80).all(|x| terminal.backend().buffer()[(x, 5)].symbol() == " "));
        app.recent_reply_links = vec![crate::codex_usage::ReplyLink {
            url: "https://example.com".into(),
            label: Some("Preview".into()),
        }];
        draw(&app, &mut terminal);
        assert_eq!(terminal.backend().buffer()[(2, 5)].symbol(), "p");
        assert_eq!(
            terminal.backend().buffer()[(2, 5)].fg,
            reply_link_color(&app.palette)
        );
        app.mouse_capture = true;
        app.mouse_position = Some(Position::new(2, 5));
        draw(&app, &mut terminal);
        assert_eq!(terminal.backend().buffer()[(2, 5)].fg, app.palette.accent);
        app.mouse_position = Some(Position::new(1, 5));
        draw(&app, &mut terminal);
        assert_eq!(
            terminal.backend().buffer()[(2, 5)].fg,
            reply_link_color(&app.palette)
        );
        assert_eq!(
            recent_reply_link_at(&app, &runtimes, Position::new(2, 5)),
            Some("https://example.com")
        );
        assert_eq!(
            recent_reply_link_at(&app, &runtimes, Position::new(1, 5)),
            None
        );
        let rt = app
            .runtime_for_pane_in_workspace(&runtimes, 0, pane_id)
            .unwrap();
        assert_eq!(rt.visible_text(), original);
        rt.test_process_pty_bytes(b"\x1b[6;1H\x1b[2KChoose a model");
        draw(&app, &mut terminal);
        assert_eq!(terminal.backend().buffer()[(0, 5)].symbol(), "C");
        assert_eq!(
            recent_reply_link_at(&app, &runtimes, Position::new(2, 5)),
            None
        );
    }

    #[test]
    fn recognizes_only_bottom_composer_hints() {
        assert!(is_hint_footer("reply\n› Ask Codex to do anything\n\n  GPT-6 · thread\n  ← for agents · ? for shortcuts\n"));
        assert!(is_hint_footer("› prompt\n\nmodel\n? for shortcuts\n"));
        assert!(!is_hint_footer(
            "reply mentioning ? for shortcuts\n› prompt\nmodel\n"
        ));
        assert!(!is_hint_footer("Choose a model\n? for shortcuts\n"));
        assert!(!is_hint_footer("› prompt\nmodel\n? for shortcuts\nmenu\n"));
    }
    #[test]
    fn footer_links_have_independent_unicode_targets_and_plain_padding() {
        let links = vec![
            crate::codex_usage::ReplyLink {
                url: "https://example.com/a".into(),
                label: Some("最新".into()),
            },
            crate::codex_usage::ReplyLink {
                url: "https://example.com/b".into(),
                label: Some("Older".into()),
            },
        ];
        let rect = Rect::new(10, 20, 80, 1);
        let targets = link_targets(&links, rect);
        assert_eq!(targets[0].0, Rect::new(12, 20, 4, 1));
        assert_eq!(targets[1].0, Rect::new(18, 20, 5, 1));
        assert_eq!(targets[1].2, "https://example.com/b");
        assert!(!targets
            .iter()
            .any(|(r, _, _)| r.contains(Position::new(17, 20))));
        let narrow = link_targets(&links, Rect::new(0, 0, 5, 1));
        assert_eq!(narrow.len(), 1);
        assert_eq!(narrow[0].0.width, 3);
        let long_label =
            "a complete link label that is longer than the old thirty two character limit";
        let long = vec![crate::codex_usage::ReplyLink {
            url: "https://example.com/long".into(),
            label: Some(long_label.into()),
        }];
        let targets = link_targets(&long, Rect::new(0, 0, 100, 1));
        assert_eq!(targets[0].1, long_label);
        assert_eq!(targets[0].0.width, display_width_u16(long_label));
        assert!(link_targets(&[], rect).is_empty());
    }
}
