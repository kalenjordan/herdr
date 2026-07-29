use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::api::schema::{
    ErrorBody, ErrorResponse, ResponseResult, SecretRequestOutcome, SecretRequestParams,
    SuccessResponse,
};

use super::App;

impl App {
    pub(crate) fn handle_deferred_secret_request(
        &mut self,
        id: String,
        params: SecretRequestParams,
        respond_to: std::sync::mpsc::Sender<String>,
    ) -> bool {
        let Some(value) = crate::platform::read_clipboard_text() else {
            send_error(
                respond_to,
                id,
                "clipboard_unavailable",
                "could not read text from the clipboard",
            );
            return false;
        };
        self.handle_secret_request_value(id, params, respond_to, value)
    }

    fn handle_secret_request_value(
        &self,
        id: String,
        params: SecretRequestParams,
        respond_to: std::sync::mpsc::Sender<String>,
        value: String,
    ) -> bool {
        let result = self.prepare_secret_request(&params);
        let (root, path, display_path, replaces_existing) = match result {
            Ok(prepared) => prepared,
            Err((code, message)) => {
                send_error(respond_to, id, code, message);
                return false;
            }
        };
        if value.is_empty() {
            send_error(
                respond_to,
                id,
                "clipboard_empty",
                "copy a secret value to the clipboard and try again",
            );
            return false;
        }
        if value.contains(['\r', '\n', '\0']) {
            send_error(
                respond_to,
                id,
                "invalid_clipboard_secret",
                "clipboard secret must be a single line without NUL characters",
            );
            return false;
        }

        let outcome = if replaces_existing {
            SecretRequestOutcome::Replaced
        } else {
            SecretRequestOutcome::Added
        };
        let write_result = ensure_secret_path_contained(&root, &path)
            .map_err(std::io::Error::other)
            .and_then(|()| upsert_env_value(&path, &params.name, &value));
        if let Err(error) = write_result {
            send_error(
                respond_to,
                id,
                "secret_request_failed",
                format!("could not update file: {error}"),
            );
            return false;
        }

        send_success(respond_to, id, outcome, params.name, display_path);
        false
    }

    fn prepare_secret_request(
        &self,
        params: &SecretRequestParams,
    ) -> Result<(PathBuf, PathBuf, String, bool), (&'static str, String)> {
        if !valid_env_name(&params.name) {
            return Err((
                "invalid_secret_name",
                "variable name must match [A-Za-z_][A-Za-z0-9_]*".into(),
            ));
        }
        if params
            .label
            .as_ref()
            .is_some_and(|label| label.contains(['\r', '\n', '\0']) || label.chars().count() > 120)
        {
            return Err((
                "invalid_secret_label",
                "label must be one line and no longer than 120 characters".into(),
            ));
        }

        let (ws_idx, _) = self.parse_pane_id(&params.pane_id).ok_or_else(|| {
            (
                "pane_not_found",
                format!("pane {} not found", params.pane_id),
            )
        })?;
        let relative = validate_env_file(&params.file)?;
        let workspace = self
            .state
            .workspaces
            .get(ws_idx)
            .ok_or(("workspace_not_found", "workspace not found".into()))?;
        let root = workspace.identity_cwd.clone();
        let path = root.join(&relative);
        ensure_secret_path_contained(&root, &path)
            .map_err(|message| ("invalid_secret_file", message))?;
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let replaces_existing = env_contains_name(&existing, &params.name);
        Ok((
            root,
            path,
            relative.display().to_string(),
            replaces_existing,
        ))
    }
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn validate_env_file(raw: &str) -> Result<PathBuf, (&'static str, String)> {
    let path = Path::new(raw);
    let valid_components = !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    let valid_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env" || name.starts_with(".env."));
    if !valid_components || !valid_name {
        return Err((
            "invalid_secret_file",
            "file must be a relative .env or .env.* path within the pane workspace".into(),
        ));
    }
    Ok(path.to_path_buf())
}

fn ensure_secret_path_contained(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "secret file must remain inside the pane workspace".to_string())?;
    let mut candidate = root.to_path_buf();
    for component in relative.components() {
        candidate.push(component);
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata_is_link(&metadata) => {
                return Err(format!(
                    "refusing symbolic link in secret file path: {}",
                    candidate.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect secret file path {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_link(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link(metadata: &std::fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn env_line_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (name, _) = line.split_once('=')?;
    Some(name.trim_end())
}

fn env_contains_name(contents: &str, name: &str) -> bool {
    contents
        .lines()
        .any(|line| env_line_name(line) == Some(name))
}

fn upsert_env_value(path: &Path, name: &str, value: &str) -> std::io::Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error),
    };
    let replacement = format!("{name}={value}");
    let mut found = false;
    let mut lines = Vec::new();
    for line in existing.lines() {
        if env_line_name(line) == Some(name) {
            if !found {
                lines.push(replacement.clone());
                found = true;
            }
        } else {
            lines.push(line.to_string());
        }
    }
    if !found {
        lines.push(replacement);
    }
    let mut updated = lines.join("\n");
    updated.push('\n');

    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("secret file has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temp_path = parent.join(format!(
        ".herdr-secret-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp_path)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        file.set_permissions(metadata.permissions())?;
    }
    if let Err(error) = file
        .write_all(updated.as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| crate::platform::replace_file(&temp_path, path))
    {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

fn send_success(
    respond_to: std::sync::mpsc::Sender<String>,
    id: String,
    outcome: SecretRequestOutcome,
    name: String,
    file: String,
) {
    let response = serde_json::to_string(&SuccessResponse {
        id,
        result: ResponseResult::SecretRequest {
            outcome,
            name,
            file,
        },
    })
    .unwrap_or_else(|_| "{}".to_string());
    let _ = respond_to.send(response);
}

fn send_error(
    respond_to: std::sync::mpsc::Sender<String>,
    id: String,
    code: impl Into<String>,
    message: impl Into<String>,
) {
    let response = serde_json::to_string(&ErrorResponse {
        id,
        error: ErrorBody {
            code: code.into(),
            message: message.into(),
        },
    })
    .unwrap_or_else(|_| "{}".to_string());
    let _ = respond_to.send(response);
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn unique_temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "herdr-secret-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }

    #[test]
    fn env_name_validation_is_strict() {
        assert!(valid_env_name("STRIPE_SECRET_KEY"));
        assert!(valid_env_name("_TOKEN_2"));
        assert!(!valid_env_name("2_TOKEN"));
        assert!(!valid_env_name("TOKEN-NAME"));
    }

    #[test]
    fn env_file_must_be_relative_and_dot_env_named() {
        assert_eq!(
            validate_env_file(".env").expect("valid"),
            PathBuf::from(".env")
        );
        assert!(validate_env_file("config/.env.local").is_ok());
        assert!(validate_env_file("../.env").is_err());
        assert!(validate_env_file("/tmp/.env").is_err());
        assert!(validate_env_file("secrets.txt").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn secret_path_rejects_intermediate_symbolic_link() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_path("symlink-root");
        let outside = unique_temp_path("symlink-outside");
        std::fs::create_dir_all(&root).expect("root");
        std::fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, root.join("linked")).expect("symlink");

        let error = ensure_secret_path_contained(&root, &root.join("linked/.env"))
            .expect_err("intermediate symlink must be rejected");

        assert!(error.contains("symbolic link"));
        std::fs::remove_dir_all(root).expect("root cleanup");
        std::fs::remove_dir_all(outside).expect("outside cleanup");
    }

    #[test]
    fn secret_request_adds_value_and_returns_only_metadata() {
        let directory = unique_temp_path("request-add");
        std::fs::create_dir_all(&directory).expect("directory");
        let mut workspace = crate::workspace::Workspace::test_new("secret");
        workspace.identity_cwd = directory.clone();
        let pane_id = format!("{}:p1", workspace.id);
        let mut app = test_app();
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        let (respond_to, response_rx) = std::sync::mpsc::channel();

        assert!(!app.handle_secret_request_value(
            "secret".into(),
            SecretRequestParams {
                name: "OPENAI_API_KEY".into(),
                pane_id,
                file: ".env".into(),
                label: Some("OpenAI API key".into()),
            },
            respond_to,
            "test-value".into(),
        ));

        let response_text = response_rx.recv().expect("success response");
        assert!(!response_text.contains("test-value"));
        let response: SuccessResponse = serde_json::from_str(&response_text).expect("json");
        assert_eq!(
            response.result,
            ResponseResult::SecretRequest {
                outcome: SecretRequestOutcome::Added,
                name: "OPENAI_API_KEY".into(),
                file: ".env".into(),
            }
        );
        assert_eq!(
            std::fs::read_to_string(directory.join(".env")).expect("env"),
            "OPENAI_API_KEY=test-value\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(directory.join(".env"))
                    .expect("metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn invalid_clipboard_value_is_rejected_without_exposure() {
        let directory = unique_temp_path("request-invalid-clipboard");
        std::fs::create_dir_all(&directory).expect("directory");
        let mut workspace = crate::workspace::Workspace::test_new("secret");
        workspace.identity_cwd = directory.clone();
        let pane_id = format!("{}:p1", workspace.id);
        let mut app = test_app();
        app.state.workspaces = vec![workspace];
        app.state.active = Some(0);
        let (respond_to, response_rx) = std::sync::mpsc::channel();

        assert!(!app.handle_secret_request_value(
            "secret".into(),
            SecretRequestParams {
                name: "TOKEN".into(),
                pane_id,
                file: ".env".into(),
                label: None,
            },
            respond_to,
            "must-not-leak\nsecond-line".into(),
        ));

        let response_text = response_rx.recv().expect("error response");
        assert!(!response_text.contains("must-not-leak"));
        let response: ErrorResponse = serde_json::from_str(&response_text).expect("json");
        assert_eq!(response.error.code, "invalid_clipboard_secret");
        assert!(!directory.join(".env").exists());
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn env_upsert_replaces_one_assignment_without_exposing_other_values() {
        let directory = std::env::temp_dir().join(format!(
            "herdr-secret-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("tempdir");
        let path = directory.join(".env");
        std::fs::write(&path, "OTHER=keep\nexport TOKEN=old\nTOKEN=duplicate\n").expect("fixture");

        upsert_env_value(&path, "TOKEN", "new").expect("upsert");

        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "OTHER=keep\nTOKEN=new\n"
        );
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
