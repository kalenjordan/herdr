use bytes::Bytes;
use ratatui::layout::Rect;

use crate::api::schema::{CodexThreadRenameCurrentParams, ResponseResult};
use crate::app::App;

use super::super::api_helpers::{encode_api_keys, encode_api_text};
use super::responses::{encode_error, encode_success};

impl App {
    pub(super) fn handle_codex_thread_rename_current(
        &mut self,
        id: String,
        params: CodexThreadRenameCurrentParams,
    ) -> String {
        let name = params.name.trim();
        if name.is_empty() {
            return encode_error(id, "invalid_name", "Codex thread name must not be empty");
        }
        if name.chars().any(char::is_control) {
            return encode_error(
                id,
                "invalid_name",
                "Codex thread name must not contain control characters",
            );
        }
        let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.caller_pane_id) else {
            return encode_error(
                id,
                "caller_pane_not_found",
                "the calling Herdr pane is no longer available",
            );
        };
        let Some(terminal_id) = self.state.terminal_id_for_pane(ws_idx, pane_id) else {
            return encode_error(
                id,
                "caller_pane_not_found",
                "the calling Herdr pane has no terminal",
            );
        };
        let is_codex = self
            .state
            .terminals
            .get(&terminal_id)
            .and_then(|terminal| terminal.effective_agent_label())
            == Some("codex");
        if !is_codex {
            return encode_error(
                id,
                "not_a_codex_thread",
                "the calling pane is not running Codex",
            );
        }
        let Some(runtime) = self.lookup_runtime_sender(ws_idx, pane_id) else {
            return encode_error(
                id,
                "caller_pane_not_found",
                "the calling Herdr pane has no terminal runtime",
            );
        };
        if !codex_composer_is_empty(runtime) {
            return encode_error(
                id,
                "codex_input_not_empty",
                "the Codex input is not empty; thread rename was skipped",
            );
        }

        let command = format!("/rename {name}");
        let mut input = encode_api_text(runtime, &command);
        let enter = match encode_api_keys(runtime, &["Enter".into()]) {
            Ok(mut keys) => keys.pop().unwrap_or_default(),
            Err(key) => {
                return encode_error(
                    id,
                    "codex_thread_rename_failed",
                    format!("unsupported key {key}"),
                );
            }
        };
        input.extend_from_slice(&enter);
        if let Err(err) = runtime.try_send_bytes(Bytes::from(input)) {
            return encode_error(
                id,
                "codex_thread_rename_failed",
                format!("failed to submit rename command: {err}"),
            );
        }

        encode_success(
            id,
            ResponseResult::CodexThreadRenamed {
                pane_id: params.caller_pane_id,
                name: name.to_string(),
            },
        )
    }
}

fn codex_composer_is_empty(runtime: &crate::terminal::TerminalRuntime) -> bool {
    let Some(cursor) = runtime.cursor_state(Rect::new(0, 0, u16::MAX, u16::MAX), true) else {
        return false;
    };

    cursor.visible && cursor.x == 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::Workspace;

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        App::new(
            &crate::config::Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        )
    }

    fn app_with_codex_session() -> (App, String) {
        let mut app = test_app();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.terminal_id_for_pane(0, pane_id).unwrap();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .detected_agent = Some(crate::detect::Agent::Codex);
        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        (app, public_pane_id)
    }

    #[tokio::test]
    async fn rename_atomically_injects_slash_command_into_calling_codex_pane() {
        let (mut app, pane_id) = app_with_codex_session();
        let internal_pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                24,
                0,
                b"\x1b[?25h\x1b[1;1H\xe2\x80\xba Summarize recent commits\x1b[1;3H",
                1,
            );
        app.state.insert_test_runtime(internal_pane_id, runtime);
        let response = app.handle_codex_thread_rename_current(
            "rename".into(),
            CodexThreadRenameCurrentParams {
                caller_pane_id: pane_id.clone(),
                name: " API cleanup ".into(),
            },
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(response["result"]["type"], "codex_thread_renamed");
        assert_eq!(response["result"]["pane_id"], pane_id);
        assert_eq!(response["result"]["name"], "API cleanup");
        assert_eq!(
            rx.try_recv().unwrap(),
            bytes::Bytes::from_static(b"/rename API cleanup\r")
        );
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rename_skips_non_empty_codex_input() {
        let (mut app, pane_id) = app_with_codex_session();
        let internal_pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let (runtime, mut rx) =
            crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
                80,
                24,
                0,
                b"\x1b[?25h\x1b[1;1H\xe2\x80\xba draft reply\x1b[1;14H",
                1,
            );
        app.state.insert_test_runtime(internal_pane_id, runtime);

        let response = app.handle_codex_thread_rename_current(
            "rename".into(),
            CodexThreadRenameCurrentParams {
                caller_pane_id: pane_id,
                name: "API cleanup".into(),
            },
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(response["error"]["code"], "codex_input_not_empty");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn rename_rejects_panes_without_codex_session() {
        let mut app = test_app();
        app.state.workspaces = vec![Workspace::test_new("test")];
        app.state.active = Some(0);
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        let response = app.handle_codex_thread_rename_current(
            "rename".into(),
            CodexThreadRenameCurrentParams {
                caller_pane_id: public_pane_id,
                name: "API cleanup".into(),
            },
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(response["error"]["code"], "not_a_codex_thread");
    }

    #[test]
    fn rename_rejects_control_characters_before_writing() {
        let (mut app, pane_id) = app_with_codex_session();
        let response = app.handle_codex_thread_rename_current(
            "rename".into(),
            CodexThreadRenameCurrentParams {
                caller_pane_id: pane_id,
                name: "safe\n/quit".into(),
            },
        );
        let response: serde_json::Value = serde_json::from_str(&response).unwrap();

        assert_eq!(response["error"]["code"], "invalid_name");
    }
}
