mod cli;
mod client;

use std::process::ExitCode;

use client::BridgeClient;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let token_file = match std::env::var("FRONA_TOKEN_FILE") {
        Ok(path) => path,
        Err(_) => {
            eprintln!("error: FRONA_TOKEN_FILE environment variable not set");
            return ExitCode::FAILURE;
        }
    };

    let api_url = match std::env::var("FRONA_API_URL") {
        Ok(url) => url,
        Err(_) => {
            eprintln!("error: FRONA_API_URL environment variable not set");
            return ExitCode::FAILURE;
        }
    };

    let token = match std::fs::read_to_string(&token_file) {
        Ok(t) => t.trim().to_string(),
        Err(e) => {
            eprintln!("error: failed to read token file '{token_file}': {e}");
            return ExitCode::FAILURE;
        }
    };

    let client = BridgeClient::new(api_url, token);
    let args: Vec<String> = std::env::args().collect();

    cli::run(client, args).await
}
