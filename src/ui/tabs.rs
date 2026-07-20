use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::Paragraph,
    Frame,
};

use super::text::display_width_u16;
use super::widgets::panel_contrast_fg;
use crate::app::AppState;

const MIN_TAB_WIDTH: u16 = 8;
const NEW_TAB_WIDTH: u16 = 3;
const TAB_SCROLL_BUTTON_WIDTH: u16 = 3;
#[cfg(test)]
pub(crate) fn tab_content_rect(ws: &crate::workspace::Workspace, area: Rect) -> Rect {
    tab_content_rect_with_status(ws, &[], None, area)
}

pub(crate) fn tab_content_rect_with_status(
    ws: &crate::workspace::Workspace,
    plugin_items: &[crate::plugin_status::PluginStatusItem],
    context_used_percent: Option<u8>,
    area: Rect,
) -> Rect {
    let reserved = status_labels(ws, plugin_items, context_used_percent)
        .iter()
        .map(|label| display_width_u16(label).saturating_add(2))
        .sum::<u16>()
        .min(area.width);
    Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(reserved),
        area.height,
    )
}

fn git_status_label(ws: &crate::workspace::Workspace) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(count) = ws.git_dirty_count().filter(|count| *count > 0) {
        parts.push(format!("●{count}"));
    }
    if let Some((ahead, behind)) = ws.git_ahead_behind() {
        if ahead > 0 {
            parts.push(format!("↑{ahead}"));
        }
        if behind > 0 {
            parts.push(format!("↓{behind}"));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn context_usage_label(used: u8) -> String {
    format!("{}%", used.min(100))
}

fn status_labels(
    ws: &crate::workspace::Workspace,
    plugin_items: &[crate::plugin_status::PluginStatusItem],
    context_used_percent: Option<u8>,
) -> Vec<String> {
    let mut labels = plugin_items
        .iter()
        .map(|item| item.label.clone())
        .collect::<Vec<_>>();
    if let Some(git) = git_status_label(ws) {
        labels.push(git);
    }
    if let Some(used) = context_used_percent {
        labels.push(context_usage_label(used));
    }
    labels
}

fn right_status_width(
    ws: &crate::workspace::Workspace,
    plugin_items: &[crate::plugin_status::PluginStatusItem],
    area: Rect,
) -> u16 {
    status_labels(ws, plugin_items, None)
        .iter()
        .map(|label| display_width_u16(label).saturating_add(2))
        .sum::<u16>()
        .min(area.width)
}

fn status_rect(
    ws: &crate::workspace::Workspace,
    plugin_items: &[crate::plugin_status::PluginStatusItem],
    area: Rect,
) -> Rect {
    let width = right_status_width(ws, plugin_items, area);
    Rect::new(
        area.x + area.width.saturating_sub(width),
        area.y,
        width,
        area.height,
    )
}

fn context_usage_rect(app: &AppState, status_rect: Rect) -> Rect {
    let Some(used) = app.context_used_percent else {
        return Rect::default();
    };
    let tabs_end = if app.mouse_capture && app.view.new_tab_hit_area.width > 0 {
        app.view.new_tab_hit_area.x + app.view.new_tab_hit_area.width
    } else {
        trailing_tab_controls_x(&app.view.tab_hit_areas, app.view.tab_bar_rect.x)
    };
    let right = status_rect.x;
    Rect::new(
        tabs_end.min(right),
        app.view.tab_bar_rect.y,
        display_width_u16(&context_usage_label(used))
            .saturating_add(2)
            .min(right.saturating_sub(tabs_end)),
        1,
    )
}

#[derive(Debug, Clone, Default)]
pub(crate) struct TabBarView {
    pub scroll: usize,
    pub tab_hit_areas: Vec<Rect>,
    pub scroll_left_hit_area: Rect,
    pub scroll_right_hit_area: Rect,
    pub new_tab_hit_area: Rect,
}

fn tab_width(ws: &crate::workspace::Workspace, tab_idx: usize) -> u16 {
    display_width_u16(&tab_chrome_label(ws, tab_idx))
        .saturating_add(4)
        .max(MIN_TAB_WIDTH)
}

fn tab_chrome_label(ws: &crate::workspace::Workspace, tab_idx: usize) -> String {
    let name = ws
        .tab_display_name(tab_idx)
        .unwrap_or_else(|| (tab_idx + 1).to_string());
    if ws.tabs.get(tab_idx).is_some_and(|tab| tab.zoomed) {
        format!("{name} Z")
    } else {
        name
    }
}

fn layout_tab_hit_areas(ws: &crate::workspace::Workspace, area: Rect, scroll: usize) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); ws.tabs.len()];
    if area.width == 0 || area.height == 0 {
        return rects;
    }

    let mut x = area.x;
    let right = area.x + area.width;
    for (idx, rect) in rects.iter_mut().enumerate().skip(scroll) {
        if x >= right {
            break;
        }
        let desired = tab_width(ws, idx);
        let remaining = right.saturating_sub(x);
        let width = desired.min(remaining).max(1);
        *rect = Rect::new(x, area.y, width, 1);
        x = x.saturating_add(width + 1);
    }
    rects
}

fn centered_tab_scroll(ws: &crate::workspace::Workspace, area: Rect) -> usize {
    let mut best_scroll = ws.active_tab;
    let mut best_distance = u16::MAX;
    let viewport_center = area.x.saturating_mul(2).saturating_add(area.width);

    for scroll in 0..=ws.active_tab {
        let rects = layout_tab_hit_areas(ws, area, scroll);
        let Some(active_rect) = rects.get(ws.active_tab).copied() else {
            continue;
        };
        if active_rect.width == 0 {
            continue;
        }

        let active_center = active_rect
            .x
            .saturating_mul(2)
            .saturating_add(active_rect.width);
        let distance = active_center.abs_diff(viewport_center);
        if distance <= best_distance {
            best_distance = distance;
            best_scroll = scroll;
        }
    }

    best_scroll
}

fn trailing_tab_controls_x(tab_hit_areas: &[Rect], fallback_x: u16) -> u16 {
    tab_hit_areas
        .iter()
        .rev()
        .find(|rect| rect.width > 0)
        .map(|rect| rect.x + rect.width)
        .unwrap_or(fallback_x)
}

fn max_tab_scroll(ws: &crate::workspace::Workspace, area: Rect) -> usize {
    (0..ws.tabs.len())
        .find(|&scroll| {
            layout_tab_hit_areas(ws, area, scroll)
                .last()
                .is_some_and(|rect| rect.width > 0)
        })
        .unwrap_or(0)
}

pub(crate) fn compute_tab_bar_view(
    ws: &crate::workspace::Workspace,
    area: Rect,
    current_scroll: usize,
    follow_active: bool,
    mouse_chrome: bool,
) -> TabBarView {
    if area.width == 0 || area.height == 0 {
        return TabBarView::default();
    }

    if !mouse_chrome {
        let max_scroll = max_tab_scroll(ws, area);
        let scroll = if follow_active {
            centered_tab_scroll(ws, area).min(max_scroll)
        } else {
            current_scroll.min(max_scroll)
        };
        return TabBarView {
            scroll,
            tab_hit_areas: layout_tab_hit_areas(ws, area, scroll),
            scroll_left_hit_area: Rect::default(),
            scroll_right_hit_area: Rect::default(),
            new_tab_hit_area: Rect::default(),
        };
    }

    let area_right = area.x + area.width;
    let all_tabs_area = Rect::new(
        area.x,
        area.y,
        area.width.saturating_sub(NEW_TAB_WIDTH),
        area.height,
    );
    let all_tabs = layout_tab_hit_areas(ws, all_tabs_area, 0);
    let overflow = all_tabs.iter().any(|rect| rect.width == 0);
    if !overflow {
        let new_tab_x = trailing_tab_controls_x(&all_tabs, area.x);
        let new_tab_hit_area = Rect::new(
            new_tab_x,
            area.y,
            area_right.saturating_sub(new_tab_x).min(NEW_TAB_WIDTH),
            1,
        );
        return TabBarView {
            scroll: 0,
            tab_hit_areas: all_tabs,
            scroll_left_hit_area: Rect::default(),
            scroll_right_hit_area: Rect::default(),
            new_tab_hit_area,
        };
    }

    let left_hit_area = Rect::new(area.x, area.y, TAB_SCROLL_BUTTON_WIDTH.min(area.width), 1);
    let tab_area_x = left_hit_area.x + left_hit_area.width;
    let reserved_trailing_width = NEW_TAB_WIDTH.saturating_add(TAB_SCROLL_BUTTON_WIDTH);
    let tab_area_right = area_right.saturating_sub(reserved_trailing_width);
    let tab_area = Rect::new(
        tab_area_x,
        area.y,
        tab_area_right.saturating_sub(tab_area_x),
        area.height,
    );

    let max_scroll = max_tab_scroll(ws, tab_area);
    let scroll = if follow_active {
        centered_tab_scroll(ws, tab_area).min(max_scroll)
    } else {
        current_scroll.min(max_scroll)
    };
    let tab_hit_areas = layout_tab_hit_areas(ws, tab_area, scroll);
    let trailing_x = trailing_tab_controls_x(&tab_hit_areas, tab_area_x).min(tab_area_right);
    let right_hit_area = Rect::new(
        trailing_x,
        area.y,
        area_right
            .saturating_sub(trailing_x)
            .min(TAB_SCROLL_BUTTON_WIDTH),
        1,
    );
    let new_tab_x = right_hit_area.x + right_hit_area.width;
    let new_tab_hit_area = Rect::new(
        new_tab_x,
        area.y,
        area_right.saturating_sub(new_tab_x).min(NEW_TAB_WIDTH),
        1,
    );

    TabBarView {
        scroll,
        tab_hit_areas,
        scroll_left_hit_area: left_hit_area,
        scroll_right_hit_area: right_hit_area,
        new_tab_hit_area,
    }
}

fn tab_drop_indicator_x(
    app: &AppState,
    ws: &crate::workspace::Workspace,
    insert_idx: usize,
) -> Option<u16> {
    let mut visible_tabs = app
        .view
        .tab_hit_areas
        .iter()
        .enumerate()
        .filter(|(_, rect)| rect.width > 0);
    let first_visible = visible_tabs.clone().next()?;
    let last_visible = visible_tabs.next_back().unwrap_or(first_visible);

    if insert_idx == 0 {
        return Some(if first_visible.0 == 0 {
            first_visible.1.x
        } else {
            app.view.tab_scroll_left_hit_area.x + app.view.tab_scroll_left_hit_area.width
        });
    }

    if let Some((_, rect)) = app
        .view
        .tab_hit_areas
        .iter()
        .enumerate()
        .find(|(idx, rect)| *idx == insert_idx && rect.width > 0)
    {
        return Some(rect.x.saturating_sub(1));
    }

    if insert_idx >= ws.tabs.len() {
        return Some(if last_visible.0 + 1 >= ws.tabs.len() {
            last_visible.1.x + last_visible.1.width
        } else {
            app.view.tab_scroll_right_hit_area.x.saturating_sub(1)
        });
    }

    None
}

pub(super) fn render_tab_bar(app: &AppState, frame: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(active_ws_idx) = app.active else {
        return;
    };
    let Some(ws) = app.workspaces.get(active_ws_idx) else {
        return;
    };
    let p = &app.palette;

    frame.render_widget(
        Paragraph::new(" ".repeat(area.width as usize)).style(Style::default().bg(p.panel_bg)),
        area,
    );

    let status_rect = status_rect(ws, &app.plugin_status_items, area);
    if status_rect.width > 0 {
        let mut spans = app
            .plugin_status_items
            .iter()
            .map(|item| {
                ratatui::text::Span::styled(
                    format!(" {} ", item.label),
                    Style::default().fg(p.overlay1).bg(p.panel_bg),
                )
            })
            .collect::<Vec<_>>();
        if let Some(label) = git_status_label(ws) {
            spans.push(ratatui::text::Span::styled(
                format!(" {label} "),
                Style::default().fg(p.overlay1).bg(p.panel_bg),
            ));
        }
        frame.render_widget(
            Paragraph::new(ratatui::text::Line::from(spans)).right_aligned(),
            status_rect,
        );
    }

    if let Some(used) = app.context_used_percent {
        let rect = context_usage_rect(app, status_rect);
        if rect.width > 0 {
            let color = if used > 50 { p.peach } else { p.overlay1 };
            frame.render_widget(
                Paragraph::new(format!(" {} ", context_usage_label(used)))
                    .style(Style::default().fg(color).bg(p.panel_bg)),
                rect,
            );
        }
    }

    let first_visible_idx = app
        .view
        .tab_hit_areas
        .iter()
        .enumerate()
        .find(|(_, rect)| rect.width > 0)
        .map(|(idx, _)| idx);
    let last_visible_idx = app
        .view
        .tab_hit_areas
        .iter()
        .enumerate()
        .rev()
        .find(|(_, rect)| rect.width > 0)
        .map(|(idx, _)| idx);
    let can_scroll_left = app.view.tab_scroll_left_hit_area.width > 0 && app.tab_scroll > 0;
    let can_scroll_right = app.view.tab_scroll_right_hit_area.width > 0
        && last_visible_idx.is_some_and(|idx| idx + 1 < ws.tabs.len());

    if app.mouse_capture && app.view.tab_scroll_left_hit_area.width > 0 {
        let style = if can_scroll_left {
            Style::default().fg(p.overlay1).bg(p.surface0)
        } else {
            Style::default()
                .fg(p.overlay0)
                .bg(p.surface0)
                .add_modifier(Modifier::DIM)
        };
        frame.render_widget(
            Paragraph::new(" < ").style(style),
            app.view.tab_scroll_left_hit_area,
        );
    }

    if app.mouse_capture && app.view.tab_scroll_right_hit_area.width > 0 {
        let style = if can_scroll_right {
            Style::default().fg(p.overlay1).bg(p.surface0)
        } else {
            Style::default()
                .fg(p.overlay0)
                .bg(p.surface0)
                .add_modifier(Modifier::DIM)
        };
        frame.render_widget(
            Paragraph::new(" > ").style(style),
            app.view.tab_scroll_right_hit_area,
        );
    }

    for (idx, tab) in ws.tabs.iter().enumerate() {
        let Some(rect) = app.view.tab_hit_areas.get(idx).copied() else {
            break;
        };
        if rect.width == 0 {
            continue;
        }
        let active = idx == ws.active_tab;
        let style = if active {
            let base = Style::default().fg(panel_contrast_fg(p)).bg(p.accent);
            if tab.is_auto_named() {
                base
            } else {
                base.add_modifier(Modifier::BOLD)
            }
        } else if tab.is_auto_named() {
            Style::default()
                .fg(p.overlay0)
                .bg(p.surface0)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(p.overlay1).bg(p.surface0)
        };
        let width = rect.width as usize;
        let name = tab_chrome_label(ws, idx);
        let text = format!(" {:width$}", name, width = width.saturating_sub(1));
        frame.render_widget(Paragraph::new(text).style(style), rect);
    }

    if let Some(crate::app::state::DragState {
        target:
            crate::app::state::DragTarget::TabReorder {
                ws_idx,
                insert_idx: Some(insert_idx),
                ..
            },
    }) = &app.drag
    {
        if *ws_idx == active_ws_idx {
            if let Some(x) = tab_drop_indicator_x(app, ws, *insert_idx) {
                frame.buffer_mut()[(x.min(area.x + area.width.saturating_sub(1)), area.y)]
                    .set_symbol("│")
                    .set_style(Style::default().fg(p.accent));
            }
        }
    }

    if app.mouse_capture && app.view.new_tab_hit_area.width > 0 {
        frame.render_widget(
            Paragraph::new(" + ").style(Style::default().fg(p.overlay1)),
            app.view.new_tab_hit_area,
        );
    }

    if first_visible_idx.is_some_and(|idx| idx > 0) {
        let x = if app.mouse_capture && app.view.tab_scroll_left_hit_area.width > 0 {
            app.view.tab_scroll_left_hit_area.x + app.view.tab_scroll_left_hit_area.width
        } else {
            area.x
        };
        if x < area.x + area.width {
            frame.buffer_mut()[(x, area.y)]
                .set_symbol("…")
                .set_style(Style::default().fg(p.overlay0));
        }
    }
    if last_visible_idx.is_some_and(|idx| idx + 1 < ws.tabs.len()) {
        let x = if app.mouse_capture && app.view.tab_scroll_right_hit_area.width > 0 {
            app.view.tab_scroll_right_hit_area.x.saturating_sub(1)
        } else {
            area.x + area.width.saturating_sub(1)
        };
        if x >= area.x && x < area.x + area.width {
            frame.buffer_mut()[(x, area.y)]
                .set_symbol("…")
                .set_style(Style::default().fg(p.overlay0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::AppState;
    use crate::workspace::Workspace;
    use ratatui::{backend::TestBackend, Terminal};

    fn buffer_row_text(buffer: &ratatui::buffer::Buffer, area: Rect, row: u16) -> String {
        (area.x..area.x + area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn tab_bar_marks_zoomed_tabs_without_renaming_them() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].zoomed = true;
        let custom_tab = ws.test_add_tab(Some("test"));
        ws.tabs[custom_tab].zoomed = true;

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.view.tab_bar_rect = Rect::new(0, 0, 30, 1);
        let view = compute_tab_bar_view(&app.workspaces[0], app.view.tab_bar_rect, 0, true, false);
        app.view.tab_hit_areas = view.tab_hit_areas;

        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();

        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.contains(" 1 Z"), "tab row: {row:?}");
        assert!(row.contains(" test Z"), "tab row: {row:?}");
        assert_eq!(app.workspaces[0].tab_display_name(0).as_deref(), Some("1"));
        assert_eq!(
            app.workspaces[0].tab_display_name(custom_tab).as_deref(),
            Some("test")
        );
    }

    #[test]
    fn dirty_status_reserves_right_edge_and_renders() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        ws.cached_git_dirty_count = Some(3);
        app.active = Some(0);
        app.workspaces = vec![ws];
        app.view.tab_bar_rect = Rect::new(0, 0, 30, 1);
        let tab_area = tab_content_rect(&app.workspaces[0], app.view.tab_bar_rect);
        let view = compute_tab_bar_view(&app.workspaces[0], tab_area, 0, true, false);
        app.view.tab_hit_areas = view.tab_hit_areas;

        assert_eq!(tab_area, Rect::new(0, 0, 26, 1));
        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();

        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.ends_with(" ●3"), "tab row: {row:?}");
    }

    #[test]
    fn plugin_status_reserves_right_edge_and_renders() {
        let mut app = AppState::test_new();
        app.active = Some(0);
        app.workspaces = vec![Workspace::test_new("test")];
        app.plugin_status_items = vec![crate::plugin_status::PluginStatusItem {
            plugin_id: "example.notify".to_string(),
            id: "notifications".to_string(),
            label: "notifications off".to_string(),
            severity: crate::plugin_status::PluginStatusSeverity::Warning,
            priority: 50,
        }];
        app.view.tab_bar_rect = Rect::new(0, 0, 40, 1);
        let tab_area = tab_content_rect_with_status(
            &app.workspaces[0],
            &app.plugin_status_items,
            app.context_used_percent,
            app.view.tab_bar_rect,
        );
        assert_eq!(tab_area, Rect::new(0, 0, 21, 1));

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();
        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.ends_with(" notifications off"), "tab row: {row:?}");
    }

    #[test]
    fn context_usage_renders_as_a_percentage() {
        let mut app = AppState::test_new();
        app.active = Some(0);
        app.workspaces = vec![Workspace::test_new("test")];
        app.context_used_percent = Some(31);
        app.view.tab_bar_rect = Rect::new(0, 0, 40, 1);
        let tab_area = tab_content_rect_with_status(
            &app.workspaces[0],
            &app.plugin_status_items,
            app.context_used_percent,
            app.view.tab_bar_rect,
        );
        let view = compute_tab_bar_view(&app.workspaces[0], tab_area, 0, true, false);
        app.view.tab_hit_areas = view.tab_hit_areas;

        let backend = TestBackend::new(40, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();
        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.starts_with(" 1       31%"), "tab row: {row:?}");
    }

    #[test]
    fn context_usage_follows_mouse_tab_controls() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        for idx in 2..=6 {
            ws.test_add_tab(Some(&format!("tab-{idx}")));
        }
        app.active = Some(0);
        app.workspaces = vec![ws];
        app.context_used_percent = Some(72);
        app.mouse_capture = true;
        app.view.tab_bar_rect = Rect::new(0, 0, 30, 1);
        let tab_area = tab_content_rect_with_status(
            &app.workspaces[0],
            &app.plugin_status_items,
            app.context_used_percent,
            app.view.tab_bar_rect,
        );
        let view = compute_tab_bar_view(&app.workspaces[0], tab_area, 0, true, true);
        app.view.tab_hit_areas = view.tab_hit_areas;
        app.view.tab_scroll_left_hit_area = view.scroll_left_hit_area;
        app.view.tab_scroll_right_hit_area = view.scroll_right_hit_area;
        app.view.new_tab_hit_area = view.new_tab_hit_area;

        let context_rect = context_usage_rect(
            &app,
            status_rect(
                &app.workspaces[0],
                &app.plugin_status_items,
                app.view.tab_bar_rect,
            ),
        );
        assert_eq!(
            context_rect.x,
            app.view.new_tab_hit_area.x + app.view.new_tab_hit_area.width
        );
        assert_eq!(context_rect.width, 5);

        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();
        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.contains(" +  72%"), "tab row: {row:?}");
    }

    #[test]
    fn active_auto_named_tab_keeps_readable_weight() {
        let mut app = AppState::test_new();
        let ws = Workspace::test_new("test");

        app.workspaces = vec![ws];
        app.active = Some(0);
        app.view.tab_bar_rect = Rect::new(0, 0, 30, 1);
        let view = compute_tab_bar_view(&app.workspaces[0], app.view.tab_bar_rect, 0, true, false);
        app.view.tab_hit_areas = view.tab_hit_areas;

        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();

        let tab_rect = app.view.tab_hit_areas[0];
        let style = terminal.backend().buffer()[(tab_rect.x + 1, tab_rect.y)].style();

        assert_eq!(style.bg, Some(app.palette.accent));
        assert!(!style.add_modifier.contains(Modifier::DIM));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn zoom_marker_counts_toward_tab_width() {
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].set_custom_name("abcdefgh".into());
        ws.tabs[0].zoomed = true;

        assert_eq!(tab_width(&ws, 0), 14);
    }

    #[test]
    fn tab_width_uses_display_width_for_cjk_labels() {
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].set_custom_name("提交 herdr 的反馈".into());

        assert_eq!(
            tab_width(&ws, 0),
            display_width_u16("提交 herdr 的反馈") + 4
        );
    }

    #[test]
    fn tab_bar_renders_trailing_cjk_character() {
        let mut app = AppState::test_new();
        let mut ws = Workspace::test_new("test");
        ws.tabs[0].set_custom_name("提交 herdr 的反馈".into());

        app.active = Some(0);
        app.workspaces = vec![ws];
        app.view.tab_bar_rect = Rect::new(0, 0, 30, 1);
        let view = compute_tab_bar_view(&app.workspaces[0], app.view.tab_bar_rect, 0, true, false);
        app.view.tab_hit_areas = view.tab_hit_areas;

        let backend = TestBackend::new(30, 1);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render_tab_bar(&app, frame, app.view.tab_bar_rect))
            .unwrap();

        let row = buffer_row_text(terminal.backend().buffer(), app.view.tab_bar_rect, 0);
        assert!(row.contains('馈'), "tab row: {row:?}");
    }
}
