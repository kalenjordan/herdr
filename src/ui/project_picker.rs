use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use super::{
    text::{display_width_u16, truncate_end},
    widgets::render_panel_shell,
};
use crate::app::state::AppState;

pub(super) fn render_project_picker_overlay(app: &AppState, frame: &mut Frame) {
    let popup = app.project_picker_popup_rect();
    let Some(inner) = render_panel_shell(frame, popup, app.palette.accent, app.palette.panel_bg)
    else {
        return;
    };
    let query = app.project_picker.query.as_str();
    let placeholder = if query.is_empty() {
        "type to search projects"
    } else {
        query
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " > ",
                Style::default()
                    .fg(app.palette.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                placeholder,
                Style::default().fg(if query.is_empty() {
                    app.palette.overlay0
                } else {
                    app.palette.text
                }),
            ),
        ])),
        app.project_picker_search_rect(),
    );

    let body = app.project_picker_body_rect();
    let entries = app.project_picker_visible_entries();
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(" no matching projects")
                .style(Style::default().fg(app.palette.overlay0)),
            body,
        );
    } else {
        let start = app.project_picker.scroll.min(entries.len());
        for (visible_index, entry) in entries
            .iter()
            .skip(start)
            .take(body.height as usize)
            .enumerate()
        {
            let index = start + visible_index;
            let rect =
                ratatui::layout::Rect::new(body.x, body.y + visible_index as u16, body.width, 1);
            frame.render_widget(Clear, rect);
            let selected = index == app.project_picker.selected;
            let style = if selected {
                Style::default()
                    .bg(app.palette.accent)
                    .fg(super::widgets::panel_contrast_fg(&app.palette))
            } else {
                Style::default()
                    .bg(app.palette.panel_bg)
                    .fg(app.palette.text)
            };
            let status = if entry.workspace_idx.is_some() {
                "open"
            } else {
                "repo"
            };
            let status_width = 8usize;
            let label_width = rect.width.saturating_sub(status_width as u16 + 4) as usize;
            let workspace_name = truncate_end(&entry.name, label_width);
            let workspace_width = display_width_u16(&workspace_name) as usize;
            let tab_name = entry.tab_name.as_ref().and_then(|tab_name| {
                let remaining = label_width.saturating_sub(workspace_width.saturating_add(1));
                (remaining > 0).then(|| truncate_end(tab_name, remaining))
            });
            let tab_width = tab_name
                .as_deref()
                .map(display_width_u16)
                .unwrap_or_default() as usize;
            let rendered_label_width =
                workspace_width + tab_name.as_ref().map(|_| 1).unwrap_or_default() + tab_width;
            let tab_style = if selected {
                style
            } else {
                Style::default()
                    .bg(app.palette.panel_bg)
                    .fg(app.palette.overlay0)
            };
            let mut spans = vec![
                Span::styled("  ", style),
                Span::styled(workspace_name, style),
            ];
            if let Some(tab_name) = tab_name {
                spans.push(Span::styled(" ", style));
                spans.push(Span::styled(tab_name, tab_style));
            }
            spans.push(Span::styled(
                format!(
                    "{status:>width$}",
                    width = rect
                        .width
                        .saturating_sub(2)
                        .saturating_sub(rendered_label_width as u16)
                        as usize
                ),
                style,
            ));
            frame.render_widget(Paragraph::new(Line::from(spans)).style(style), rect);
        }
    }
    let footer = app.project_picker_footer_rect();
    let footer_text = app
        .project_picker
        .error
        .as_deref()
        .unwrap_or("enter open  esc close");
    frame.render_widget(
        Paragraph::new(format!(" {footer_text}")).style(Style::default().fg(
            if app.project_picker.error.is_some() {
                app.palette.red
            } else {
                app.palette.overlay0
            },
        )),
        footer,
    );
    let _ = inner;
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, layout::Rect, Terminal};

    use super::render_project_picker_overlay;
    use crate::app::state::{AppState, ProjectPickerEntry};

    #[test]
    fn workspace_and_tab_render_without_slash_and_with_muted_tab() {
        let mut app = AppState::test_new();
        app.view.sidebar_rect = Rect::default();
        app.view.terminal_area = Rect::new(0, 0, 80, 24);
        app.project_picker.entries = vec![
            ProjectPickerEntry {
                name: "herdr".into(),
                tab_name: Some("release-notes".into()),
                path: "/repos/herdr".into(),
                workspace_idx: Some(0),
                tab_idx: Some(0),
            },
            ProjectPickerEntry {
                name: "selected".into(),
                tab_name: None,
                path: "/repos/selected".into(),
                workspace_idx: None,
                tab_idx: None,
            },
        ];
        app.project_picker.selected = 1;

        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        terminal
            .draw(|frame| render_project_picker_overlay(&app, frame))
            .expect("render project picker");

        let row_y = app.project_picker_body_rect().y;
        let buffer = terminal.backend().buffer();
        let row = (0..buffer.area.width)
            .map(|x| buffer[(x, row_y)].symbol())
            .collect::<String>();
        assert!(row.contains("herdr release-notes"), "rendered row: {row:?}");
        assert!(!row.contains("herdr / release-notes"));
        let tab_x = row.find("release-notes").expect("rendered tab") as u16;
        assert_eq!(buffer[(tab_x, row_y)].fg, app.palette.overlay0);
    }
}
