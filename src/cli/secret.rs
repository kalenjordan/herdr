use crate::api::schema::{Method, Request, SecretRequestParams};

pub(super) fn run_secret_command(args: &[String]) -> std::io::Result<i32> {
    match args.first().map(String::as_str) {
        Some("request") => request_secret(&args[1..]),
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

fn request_secret(args: &[String]) -> std::io::Result<i32> {
    let params = match parse_request_args(args) {
        Ok(params) => params,
        Err(message) => {
            eprintln!("{message}");
            print_help();
            return Ok(2);
        }
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:secret:request".into(),
        method: Method::SecretRequest(params),
    })?)
}

fn parse_request_args(args: &[String]) -> Result<SecretRequestParams, String> {
    let name = args
        .first()
        .filter(|value| !matches!(value.as_str(), "help" | "--help" | "-h"))
        .cloned()
        .ok_or_else(|| "missing secret variable name".to_string())?;
    let pane_id = std::env::var(crate::integration::HERDR_PANE_ID_ENV_VAR)
        .map_err(|_| "secret requests must run inside a Herdr pane".to_string())?;
    let mut file = ".env".to_string();
    let mut label = None;
    let mut index = 1;
    while index < args.len() {
        let option = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("missing value for {option}"))?;
        match option {
            "--file" => file = value.clone(),
            "--label" => label = Some(value.clone()),
            _ => return Err(format!("unknown option: {option}")),
        }
        index += 2;
    }

    Ok(SecretRequestParams {
        name,
        pane_id,
        file,
        label,
    })
}

fn print_help() {
    eprintln!("herdr secret commands:");
    eprintln!("  herdr secret request <VARIABLE_NAME> [--file .env] [--label TEXT]");
}
