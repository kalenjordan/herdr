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
        discover_project_entries(&self.state, &self.terminal_runtimes)
    }

    pub(crate) fn handle_project_picker_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.close_project_picker(),
            KeyCode::Enter => self.accept_project_picker_selection(),
            KeyCode::Backspace => {
                self.state.project_picker.query.pop();
                self.state.reset_project_picker_search();
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
                self.state.reset_project_picker_search();
            }
            KeyCode::Char(character)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.state.project_picker.query.push(character);
                self.state.reset_project_picker_search();
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

    fn ensure_project_picker_selection_visible(&mut self) {
        let viewport = self.state.project_picker_body_rect().height as usize;
        let count = self.state.filtered_project_indices().len();
        self.state.project_picker.scroll = self
            .state
            .project_picker
            .scroll
            .min(count.saturating_sub(viewport));
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
            if let Some(tab_idx) = entry.tab_idx {
                if let Some(tab_id) = self.public_tab_id(workspace_idx, tab_idx) {
                    self.runtime_tab_focus("tui.project.tab.focus", tab_id);
                    self.close_project_picker();
                    return;
                }
            }
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
    pub(crate) fn reset_project_picker_search(&mut self) {
        self.project_picker.selected = 0;
        self.project_picker.scroll = 0;
        self.project_picker.error = None;
    }

    pub(crate) fn filtered_project_indices(&self) -> Vec<usize> {
        let query = normalize_search(&self.project_picker.query);
        self.project_picker
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                query.is_empty()
                    || normalize_search(&entry.name).contains(&query)
                    || entry
                        .tab_name
                        .as_ref()
                        .is_some_and(|name| normalize_search(name).contains(&query))
                    || normalize_search(&entry.path.to_string_lossy()).contains(&query)
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

fn discover_project_entries(
    state: &AppState,
    terminal_runtimes: &crate::terminal::TerminalRuntimeRegistry,
) -> Vec<ProjectPickerEntry> {
    let mut entries = Vec::new();
    for (workspace_idx, workspace) in state.workspaces.iter().enumerate() {
        let path = canonical(&workspace.identity_cwd);
        let name = workspace.display_name_from(&state.terminals, terminal_runtimes);
        for (tab_idx, _) in workspace.tabs.iter().enumerate() {
            entries.push(ProjectPickerEntry {
                name: name.clone(),
                tab_name: workspace.tab_display_name(tab_idx),
                path: path.clone(),
                workspace_idx: Some(workspace_idx),
                tab_idx: Some(tab_idx),
            });
        }
    }
    let mut seen_paths = std::collections::HashSet::new();
    for path in configured_project_paths(&state.project_directories) {
        let name = project_name(&path);
        let path = canonical(&path);
        if seen_paths.insert(path.clone()) {
            entries.push(ProjectPickerEntry {
                name,
                tab_name: None,
                path,
                workspace_idx: None,
                tab_idx: None,
            });
        }
    }
    sort_project_entries(&mut entries, &state.project_priorities);
    entries
}

fn sort_project_entries(entries: &mut [ProjectPickerEntry], priorities: &[PathBuf]) {
    entries.sort_by_key(|entry| {
        (
            priorities
                .iter()
                .position(|path| *path == entry.path)
                .unwrap_or(usize::MAX),
            entry.workspace_idx.is_none(),
            entry.name.to_lowercase(),
            entry.tab_idx,
        )
    });
}

pub(super) fn priorities_from_config(configured: &[String]) -> Vec<PathBuf> {
    configured
        .iter()
        .filter(|path| !path.trim().is_empty())
        .map(|path| canonical(&crate::worktree::expand_tilde_path(path)))
        .collect()
}

fn normalize_search(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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

fn configured_project_paths(directories: &[PathBuf]) -> Vec<PathBuf> {
    directories
        .iter()
        .filter_map(|directory| std::fs::read_dir(directory).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrowing_project_search_resets_scrolled_selection() {
        let mut state = AppState::test_new();
        state.project_picker.entries = (0..30)
            .map(|index| ProjectPickerEntry {
                name: format!("outbound-stage-{index}"),
                tab_name: None,
                path: format!("/repos/outbound-stage-{index}").into(),
                workspace_idx: None,
                tab_idx: None,
            })
            .chain(std::iter::once(ProjectPickerEntry {
                name: "outbound-dash".into(),
                tab_name: None,
                path: "/repos/outbound-dash".into(),
                workspace_idx: None,
                tab_idx: None,
            }))
            .collect();
        state.project_picker.selected = 29;
        state.project_picker.scroll = 20;
        state.project_picker.query = "outbound".into();
        state.reset_project_picker_search();
        assert_eq!(state.filtered_project_indices().len(), 31);
        assert_eq!(state.project_picker.selected, 0);
        assert_eq!(state.project_picker.scroll, 0);

        state.project_picker.selected = 30;
        state.project_picker.scroll = 20;
        state.project_picker.query = "outbound-d".into();
        state.reset_project_picker_search();
        assert_eq!(state.filtered_project_indices(), vec![30]);
        assert_eq!(state.project_picker.selected, 0);
        assert_eq!(state.project_picker.scroll, 0);
        assert_eq!(
            state.project_picker_visible_entries()[0].name,
            "outbound-dash"
        );
    }

    #[test]
    fn small_project_result_lists_fit_without_scrolling() {
        let mut state = AppState::test_new();
        state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);
        for count in 1..=15 {
            state.project_picker.entries = (0..count)
                .map(|index| ProjectPickerEntry {
                    name: format!("project-{index}"),
                    tab_name: None,
                    path: format!("/repos/project-{index}").into(),
                    workspace_idx: None,
                    tab_idx: None,
                })
                .collect();
            assert!(state.project_picker_body_rect().height as usize >= count);
        }
    }

    #[cfg(unix)]
    #[test]
    fn discovery_preserves_repo_option_alongside_open_target() {
        let root = std::env::temp_dir().join(format!(
            "herdr-project-picker-symlink-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let projects = root.join("repos");
        let target = root.join("internal-app");
        std::fs::create_dir_all(&projects).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::os::unix::fs::symlink(&target, projects.join("client-app")).unwrap();

        let mut state = AppState::test_new();
        state.workspaces.clear();
        state.project_directories = vec![projects];
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.project_picker.entries = discover_project_entries(&state, &runtimes);
        assert_eq!(state.project_picker.entries.len(), 1);
        assert_eq!(state.project_picker.entries[0].name, "client-app");
        assert_eq!(state.project_picker.entries[0].path, canonical(&target));
        state.project_picker.query = "client-app".into();
        assert_eq!(state.filtered_project_indices(), vec![0]);

        let mut workspace = crate::workspace::Workspace::test_new("Open target");
        workspace.identity_cwd = target;
        state.workspaces.push(workspace);
        state.project_picker.entries = discover_project_entries(&state, &runtimes);
        assert_eq!(state.project_picker.entries.len(), 2);
        let open = state
            .project_picker
            .entries
            .iter()
            .find(|entry| entry.workspace_idx == Some(0))
            .unwrap();
        let repo = state
            .project_picker
            .entries
            .iter()
            .find(|entry| entry.workspace_idx.is_none())
            .unwrap();
        assert_eq!(open.path, repo.path);
        assert_eq!(repo.name, "client-app");
        assert_eq!(repo.tab_idx, None);
        state.project_picker.query = "client-app".into();
        let matches = state.project_picker_visible_entries();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].workspace_idx, None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discovery_includes_every_open_workspace_outside_project_directories() {
        let mut state = AppState::test_new();
        state.workspaces = vec![
            crate::workspace::Workspace::test_new("Personal"),
            crate::workspace::Workspace::test_new("Scratch"),
        ];
        for workspace in &mut state.workspaces {
            workspace.identity_cwd = "/outside/projects/shared".into();
        }
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.project_picker.entries = discover_project_entries(&state, &runtimes);
        assert_eq!(state.project_picker.entries.len(), 2);
        for workspace_idx in 0..2 {
            assert!(state.project_picker.entries.iter().any(|entry| {
                entry.workspace_idx == Some(workspace_idx) && entry.tab_idx == Some(0)
            }));
        }
        state.project_picker.query = "scratch".into();
        let matches = state.project_picker_visible_entries();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].workspace_idx, Some(1));
    }

    #[test]
    fn renamed_open_workspace_is_searchable_by_repository_and_workspace_name() {
        let mut state = AppState::test_new();
        let mut workspace = crate::workspace::Workspace::test_new("clever");
        workspace.identity_cwd = "/repos/outbound-dash".into();
        state.workspaces = vec![workspace];
        let runtimes = crate::terminal::TerminalRuntimeRegistry::new();
        state.project_picker.entries = discover_project_entries(&state, &runtimes);
        for query in ["outbound-dash", "clever"] {
            state.project_picker.query = query.into();
            let entries = state.project_picker_visible_entries();
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].name, "clever");
            assert_eq!(entries[0].workspace_idx, Some(0));
            assert_eq!(entries[0].tab_idx, Some(0));
        }
    }

    #[test]
    fn configured_projects_include_non_git_directories() {
        let root = std::env::temp_dir().join(format!(
            "herdr-project-picker-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plain_directory = root.join("insuranceagents.site");
        let regular_file = root.join("notes.txt");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&plain_directory).unwrap();
        std::fs::write(&regular_file, "not a project directory").unwrap();

        let paths = configured_project_paths(std::slice::from_ref(&root));

        assert_eq!(paths, vec![plain_directory]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn project_filter_matches_name_and_path_case_insensitively() {
        let mut state = AppState::test_new();
        state.project_picker.entries = vec![
            ProjectPickerEntry {
                name: "Herdr".into(),
                tab_name: Some("Release notes".into()),
                path: "/repos/herdr".into(),
                workspace_idx: Some(0),
                tab_idx: Some(0),
            },
            ProjectPickerEntry {
                name: "Storefront".into(),
                tab_name: None,
                path: "/clients/acme/storefront".into(),
                workspace_idx: None,
                tab_idx: None,
            },
        ];
        state.project_picker.query = "HERD".into();
        assert_eq!(state.filtered_project_indices(), vec![0]);
        state.project_picker.query = "acme".into();
        assert_eq!(state.filtered_project_indices(), vec![1]);
        state.project_picker.query = "release".into();
        assert_eq!(state.filtered_project_indices(), vec![0]);
    }

    #[test]
    fn project_priority_promotes_preferred_repo_for_all_matching_queries() {
        let mut state = AppState::test_new();
        state.project_picker.entries = vec![
            ProjectPickerEntry {
                name: "fluidvoice".into(),
                tab_name: None,
                path: "/repos/fluidvoice".into(),
                workspace_idx: Some(0),
                tab_idx: Some(0),
            },
            ProjectPickerEntry {
                name: "voice-control".into(),
                tab_name: None,
                path: "/repos/voice-control".into(),
                workspace_idx: None,
                tab_idx: None,
            },
        ];
        state.project_priorities = vec!["/repos/voice-control".into()];
        sort_project_entries(&mut state.project_picker.entries, &state.project_priorities);

        state.project_picker.query = "voi".into();
        assert_eq!(state.filtered_project_indices(), vec![0, 1]);
        state.project_picker.query = "voice".into();
        assert_eq!(state.filtered_project_indices(), vec![0, 1]);
        state.project_picker.query = "voice control".into();
        assert_eq!(state.filtered_project_indices(), vec![0]);
        state.project_picker.query = "fluid".into();
        assert_eq!(state.filtered_project_indices(), vec![1]);
        state.project_picker.query.clear();
        assert_eq!(state.filtered_project_indices(), vec![0, 1]);
    }

    #[test]
    fn priority_rules_keep_config_order() {
        let mut state = AppState::test_new();
        state.project_picker.entries = ["voice-one", "voice-two", "voice-three"]
            .into_iter()
            .map(|name| ProjectPickerEntry {
                name: name.into(),
                tab_name: None,
                path: format!("/repos/{name}").into(),
                workspace_idx: None,
                tab_idx: None,
            })
            .collect();
        state.project_priorities = vec!["/repos/voice-three".into(), "/repos/voice-two".into()];
        sort_project_entries(&mut state.project_picker.entries, &state.project_priorities);
        state.project_picker.query = "voice".into();
        assert_eq!(state.project_picker.entries[0].name, "voice-three");
        assert_eq!(state.project_picker.entries[1].name, "voice-two");
        assert_eq!(state.filtered_project_indices(), vec![0, 1, 2]);
    }
}
