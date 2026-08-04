//! Break-glass command line surface.
//!
//! These commands operate directly on the database and are meant for the case
//! where nobody can get in through the API — the sole admin has forgotten their
//! password, or a lockout needs undoing without an authenticated caller. The
//! embedded RocksDB store allows a single writer, so the server must be stopped
//! before these run.

use std::io::{BufRead, Write};

use crate::auth::password_reset::models::PasswordResetToken;
use crate::auth::password_reset::repository::PasswordResetRepository;
use crate::auth::token::models::ApiToken;
use crate::auth::token::repository::TokenRepository;
use crate::auth::{AuthService, UserService};
use crate::core::Handle;
use crate::core::config::Config;
use crate::db::init as db;
use crate::db::repo::generic::SurrealRepo;

pub const USAGE: &str = "\
Frona server

Usage:
  frona                                    Start the server (default)
  frona reset-password [options]           Reset a user's password directly
  frona --help                             Show this message

reset-password options:
  --handle <handle>     Target user by handle
  --email <email>       Target user by email address
  --password <password> New password (omit to read it from stdin instead,
                        which keeps it out of your shell history)

The server must be stopped first: it holds an exclusive lock on the database.
Resetting a password revokes every session that user has and clears any
outstanding lockout or reset link.
";

pub struct ResetPasswordArgs {
    pub handle: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
}

pub fn parse_reset_password(args: &[String]) -> Result<ResetPasswordArgs, String> {
    let mut parsed = ResetPasswordArgs {
        handle: None,
        email: None,
        password: None,
    };

    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let take_value = |name: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match flag {
            "--handle" => {
                parsed.handle = Some(take_value("--handle")?);
                i += 2;
            }
            "--email" => {
                parsed.email = Some(take_value("--email")?);
                i += 2;
            }
            "--password" => {
                parsed.password = Some(take_value("--password")?);
                i += 2;
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }

    match (&parsed.handle, &parsed.email) {
        (None, None) => return Err("one of --handle or --email is required".into()),
        (Some(_), Some(_)) => return Err("--handle and --email are mutually exclusive".into()),
        _ => {}
    }

    Ok(parsed)
}

fn read_password_from_stdin() -> Result<String, String> {
    // No TTY echo suppression here: this is a recovery path that may well run
    // over a pipe, and pulling in a terminal crate for it isn't worth it.
    print!("New password: ");
    std::io::stdout()
        .flush()
        .map_err(|e| format!("failed to write prompt: {e}"))?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read password: {e}"))?;
    let password = line.trim_end_matches(['\r', '\n']).to_string();
    if password.is_empty() {
        return Err("password must not be empty".into());
    }
    Ok(password)
}

pub async fn run_reset_password(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_reset_password(args).map_err(|e| format!("{e}\n\n{USAGE}"))?;

    let password = match args.password {
        Some(p) => p,
        None => read_password_from_stdin()?,
    };
    AuthService::validate_password(&password)?;

    let loaded = Config::load();
    let config = loaded.config;
    let surreal = db::init(&config.database.path).await?;

    let user_service = UserService::new(SurrealRepo::new(surreal.clone()), &config.cache);
    let auth_service = AuthService::new();

    let user = match (&args.handle, &args.email) {
        (Some(handle), _) => {
            let handle = Handle::try_new(handle)?;
            user_service.find_by_handle(&handle).await?
        }
        (_, Some(email)) => {
            user_service
                .find_by_email(&AuthService::normalize_email(email))
                .await?
        }
        _ => unreachable!("parse_reset_password rejects this"),
    }
    .ok_or("no user matches that handle or email")?;

    auth_service
        .set_password(&user_service, &user.id, &password)
        .await?;

    let token_repo: SurrealRepo<ApiToken> = SurrealRepo::new(surreal.clone());
    TokenRepository::delete_by_user_id(&token_repo, &user.id).await?;

    let reset_repo: SurrealRepo<PasswordResetToken> = SurrealRepo::new(surreal.clone());
    PasswordResetRepository::delete_by_user_id(&reset_repo, &user.id).await?;

    println!(
        "Password reset for {} <{}>. All sessions revoked.",
        user.handle, user.email
    );
    println!(
        "Any lockout clears when the server restarts — it is tracked in memory, not in the database."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_handle_and_password() {
        let parsed = parse_reset_password(&args(&["--handle", "alice", "--password", "hunter22"]))
            .expect("should parse");
        assert_eq!(parsed.handle.as_deref(), Some("alice"));
        assert_eq!(parsed.password.as_deref(), Some("hunter22"));
        assert!(parsed.email.is_none());
    }

    #[test]
    fn password_is_optional() {
        let parsed = parse_reset_password(&args(&["--email", "a@b.com"])).expect("should parse");
        assert_eq!(parsed.email.as_deref(), Some("a@b.com"));
        assert!(parsed.password.is_none());
    }

    #[test]
    fn requires_a_target() {
        assert!(parse_reset_password(&args(&["--password", "hunter22"])).is_err());
    }

    #[test]
    fn rejects_both_targets() {
        assert!(
            parse_reset_password(&args(&["--handle", "alice", "--email", "a@b.com"])).is_err()
        );
    }

    #[test]
    fn rejects_unknown_flags() {
        assert!(parse_reset_password(&args(&["--nope", "x"])).is_err());
    }

    #[test]
    fn rejects_dangling_value() {
        assert!(parse_reset_password(&args(&["--handle"])).is_err());
    }
}
