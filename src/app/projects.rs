use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::schema::{Method, PaneSendInputParams, WorkspaceCreateParams};
use crate::app::state::{AppState, Mode, ProjectPickerEntry};

use super::App;

impl App {
    pub(crate) fn open_project_picker(&mut self) {
        self.state.project_picker.query.clear();
        self.state.project_picker.selected = 0;
        self.state.project_picker.scroll = 0;
        self.state.project_picker.error = None;
        self.state.project_picker.entries = self.discover_project_entries();
        self.state.mode = Mode::ProjectPicker;
    }

    fn discover_project_entries(&self) -> Vec<ProjectPickerEntry> {
        let mut paths = Vec::new();
        for directory in &self.state.project_directories {
            let Ok(children) = std::fs::read_dir(directory) else {
                continue;
            };
            for child in children.flatten() {
                let path = child.path();
                if path.is_dir() && (path.join(".git").exists() || is_bare_repository(&path)) {
                    paths.push(path);
                }
            }
        }
        paths.extend(
            self.state
                .workspaces
                .iter()
                .map(|workspace| workspace.identity_cwd.clone()),
        );
        paths.sort_by_key(|path| project_name(path).to_lowercase());
        paths.dedup_by(|left, right| canonical(left) == canonical(right));

        let mut entries: Vec<_> = paths
            .into_iter()
            .map(|path| {
                let canonical_path = canonical(&path);
                let workspace_idx = self
                    .state
                    .workspaces
                    .iter()
                    .position(|workspace| canonical(&workspace.identity_cwd) == canonical_path);
                ProjectPickerEntry {
                    name: workspace_idx
                        .and_then(|idx| self.state.workspaces.get(idx))
                        .map(|workspace| {
                            workspace
                                .display_name_from(&self.state.terminals, &self.terminal_runtimes)
                        })
                        .unwrap_or_else(|| project_name(&path)),
                    path: canonical_path,
                    workspace_idx,
                }
            })
            .collect();
        entries.sort_by_key(|entry| (entry.workspace_idx.is_none(), entry.name.to_lowercase()));
        entries
    }

    pub(crate) fn handle_project_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_project_picker(),
            KeyCode::Enter => self.accept_project_picker_selection(),
            KeyCode::Backspace => {
                self.state.project_picker.query.pop();
                self.clamp_project_picker_selection();
            }
            KeyCode::Up => self.move_project_picker_selection(-1),
            KeyCode::Down => self.move_project_picker_selection(1),
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                self.move_project_picker_selection(1)
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                self.move_project_picker_selection(-1)
            }
            KeyCode::Char('u') if key.modifiers == KeyModifiers::CONTROL => {
                self.state.project_picker.query.clear();
                self.clamp_project_picker_selection();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.state.project_picker.query.push(character);
                self.clamp_project_picker_selection();
            }
            _ => {}
        }
    }

    pub(super) fn move_project_picker_selection(&mut self, delta: isize) {
        let count = self.state.filtered_project_indices().len();
        if count == 0 {
            self.state.project_picker.selected = 0;
            return;
        }
        self.state.project_picker.selected = (self.state.project_picker.selected as isize + delta)
            .clamp(0, count as isize - 1) as usize;
        self.ensure_project_picker_selection_visible();
    }

    fn clamp_project_picker_selection(&mut self) {
        let count = self.state.filtered_project_indices().len();
        self.state.project_picker.selected = self
            .state
            .project_picker
            .selected
            .min(count.saturating_sub(1));
        self.state.project_picker.error = None;
        self.ensure_project_picker_selection_visible();
    }

    fn ensure_project_picker_selection_visible(&mut self) {
        let viewport = self.state.project_picker_body_rect().height as usize;
        if viewport == 0 {
            self.state.project_picker.scroll = 0;
        } else if self.state.project_picker.selected < self.state.project_picker.scroll {
            self.state.project_picker.scroll = self.state.project_picker.selected;
        } else if self.state.project_picker.selected >= self.state.project_picker.scroll + viewport
        {
            self.state.project_picker.scroll = self
                .state
                .project_picker
                .selected
                .saturating_add(1)
                .saturating_sub(viewport);
        }
    }

    fn close_project_picker(&mut self) {
        self.state.mode = Mode::Terminal;
        self.state.project_picker.error = None;
    }

    pub(super) fn accept_project_picker_selection(&mut self) {
        let Some(entry_index) = self
            .state
            .filtered_project_indices()
            .get(self.state.project_picker.selected)
            .copied()
        else {
            return;
        };
        let entry = self.state.project_picker.entries[entry_index].clone();
        if let Some(workspace_idx) = entry.workspace_idx {
            let workspace_id = self.public_workspace_id(workspace_idx);
            self.runtime_workspace_focus("tui.project.focus", workspace_id);
            self.close_project_picker();
            return;
        }

        let response = self.runtime_workspace_create(
            "tui.project.open",
            WorkspaceCreateParams {
                cwd: Some(entry.path.display().to_string()),
                focus: true,
                label: Some(entry.name),
                env: Default::default(),
            },
        );
        let parsed: serde_json::Value = match serde_json::from_str(&response) {
            Ok(parsed) => parsed,
            Err(error) => {
                self.state.project_picker.error = Some(format!("failed to open project: {error}"));
                return;
            }
        };
        if let Some(message) = parsed
            .pointer("/error/message")
            .and_then(|value| value.as_str())
        {
            self.state.project_picker.error = Some(message.to_string());
            return;
        }
        let Some(pane_id) = parsed
            .pointer("/result/root_pane/pane_id")
            .and_then(|value| value.as_str())
        else {
            self.state.project_picker.error = Some("project opened without a root pane".into());
            return;
        };
        let command = self.state.project_command.trim().to_string();
        if !command.is_empty() {
            self.dispatch_runtime_mutation(
                "tui.project.command",
                Method::PaneSendInput(PaneSendInputParams {
                    pane_id: pane_id.to_string(),
                    text: command,
                    keys: vec!["enter".into()],
                }),
            );
        }
        self.close_project_picker();
    }
}

impl AppState {
    pub(crate) fn filtered_project_indices(&self) -> Vec<usize> {
        let query = self.project_picker.query.trim().to_lowercase();
        self.project_picker
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || entry.name.to_lowercase().contains(&query)
                    || entry.path.to_string_lossy().to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    pub(crate) fn project_picker_visible_entries(&self) -> Vec<&ProjectPickerEntry> {
        self.filtered_project_indices()
            .into_iter()
            .filter_map(|index| self.project_picker.entries.get(index))
            .collect()
    }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn project_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn is_bare_repository(path: &Path) -> bool {
    path.join("HEAD").is_file() && path.join("objects").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_filter_matches_name_and_path_case_insensitively() {
        let mut state = AppState::test_new();
        state.project_picker.entries = vec![
            ProjectPickerEntry {
                name: "Herdr".into(),
                path: "/repos/herdr".into(),
                workspace_idx: Some(0),
            },
            ProjectPickerEntry {
                name: "Storefront".into(),
                path: "/clients/acme/storefront".into(),
                workspace_idx: None,
            },
        ];
        state.project_picker.query = "HERD".into();
        assert_eq!(state.filtered_project_indices(), vec![0]);
        state.project_picker.query = "acme".into();
        assert_eq!(state.filtered_project_indices(), vec![1]);
    }
}
