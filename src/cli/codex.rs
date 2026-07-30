use crate::api::schema::{CodexThreadRenameCurrentParams, Method};

pub(super) fn run_codex_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("rename-thread") => rename_thread(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(0)
        }
        _ => {
            print_help();
            Ok(2)
        }
    }
}

fn rename_thread(args: &[String]) -> std::io::Result<i32> {
    let Some(name_index) = args.iter().position(|arg| arg == "--current") else {
        eprintln!("usage: herdr codex rename-thread --current <name>");
        return Ok(2);
    };
    if name_index != 0 || args.len() < 2 {
        eprintln!("usage: herdr codex rename-thread --current <name>");
        return Ok(2);
    }
    let name = args[1..].join(" ");
    if name.trim().is_empty() {
        eprintln!("Codex thread name must not be empty");
        return Ok(2);
    }
    let pane_id = match std::env::var(crate::integration::HERDR_PANE_ID_ENV_VAR) {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("current Herdr pane is unavailable; run this command from a Herdr pane");
            return Ok(1);
        }
    };

    super::print_response(&super::send_request(&crate::api::schema::Request {
        id: "cli:codex:rename-thread".into(),
        method: Method::CodexThreadRenameCurrent(CodexThreadRenameCurrentParams {
            caller_pane_id: pane_id,
            name,
        }),
    })?)
}

fn print_help() {
    eprintln!("herdr codex commands:");
    eprintln!("  herdr codex rename-thread --current <name>");
}
