use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
    Frame,
};

use super::{text::truncate_end, widgets::render_panel_shell};
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
            let name = truncate_end(
                &entry.name,
                rect.width.saturating_sub(status_width as u16 + 4) as usize,
            );
            frame.render_widget(
                Paragraph::new(format!(
                    "  {name:<width$} {status}",
                    width = rect.width.saturating_sub(status_width as u16 + 3) as usize
                ))
                .style(style),
                rect,
            );
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
