//! SSO account resolution: an email is only evidence of ownership when the
//! provider verified it, and a subject only means something within its issuer.

use std::sync::Arc;

use frona::auth::oauth::models::OAuthIdentity;
use frona::auth::oauth::repository::OAuthRepository;
use frona::auth::oauth::service::{ExternalLogin, ExternalUserinfo, OAuthService, ProviderEmail};
use frona::core::error::{AppError, AuthErrorCode};
use frona::db::repo::generic::SurrealRepo;
use tower::ServiceExt;

use super::*;

const ISSUER: &str = "https://idp.example.test";

fn service(state: &AppState, match_email: bool, allow_unknown: bool) -> OAuthService {
    let mut config = (*state.config).clone();
    config.server.base_url = Some("https://frona.example.test".into());
    config.sso.authority = Some(ISSUER.into());
    config.sso.client_id = Some("client".into());
    config.sso.client_secret = Some("secret".into());
    config.sso.signups_match_email = match_email;
    config.sso.allow_unknown_email_verification = allow_unknown;
    OAuthService::new(
        &config,
        Arc::new(SurrealRepo::<OAuthIdentity>::new(state.db.clone())),
    )
    .unwrap()
}

fn repo(state: &AppState) -> SurrealRepo<OAuthIdentity> {
    SurrealRepo::new(state.db.clone())
}

fn login(issuer: &str, subject: &str, email: Option<(&str, bool)>) -> ExternalLogin {
    ExternalLogin {
        issuer: issuer.into(),
        subject: subject.into(),
        name: Some("Someone".into()),
        email: email.map(|(address, verified)| ProviderEmail {
            address: address.into(),
            verified,
        }),
    }
}

fn is_code(err: &AppError, expected: AuthErrorCode) -> bool {
    matches!(err, AppError::Auth { code, .. } if code.as_str() == expected.as_str())
}

#[tokio::test]
async fn a_verified_email_links_the_existing_account_and_ends_its_sessions() {
    let (state, _tmp) = test_app_state().await;
    let (old_token, user_id) =
        register_user(&state, "linkme", "linkme@example.com", "password123").await;
    let svc = service(&state, true, true);

    let (user, is_new) = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "sub-1", Some(("Linkme@Example.com", true))),
            None,
        )
        .await
        .unwrap();
    assert!(!is_new);
    assert_eq!(user.id, user_id);

    let identity = repo(&state)
        .find_identity(ISSUER, "sub-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(identity.user_id, user_id);
    assert_eq!(identity.issuer.as_deref(), Some(ISSUER));

    // A session held before the link (say, by whoever pre-registered the
    // address) no longer works.
    let resp = build_app(state.clone())
        .oneshot(auth_get("/api/auth/me", &old_token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_unverified_email_is_never_linked() {
    let (state, _tmp) = test_app_state().await;
    let (_t, _id) = register_user(&state, "victim", "victim@example.com", "password123").await;
    let svc = service(&state, true, true);

    // Explicitly false, and "the provider didn't say" look the same here:
    // both arrive as `verified: false`.
    let err = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "attacker", Some(("victim@example.com", false))),
            None,
        )
        .await
        .unwrap_err();
    assert!(is_code(&err, AuthErrorCode::AccountConflict), "{err}");
    assert!(
        repo(&state)
            .find_identity(ISSUER, "attacker")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_unverified_userinfo_email_is_ignored_and_a_verified_one_is_used() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "ui", "ui@example.com", "password123").await;
    let svc = service(&state, true, true);

    let bare = login(ISSUER, "sub-ui", None);
    assert!(svc.needs_userinfo(&bare));

    let unverified = ExternalUserinfo {
        email: Some(ProviderEmail {
            address: "ui@example.com".into(),
            verified: false,
        }),
        name: None,
    };
    let err = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &bare,
            Some(&unverified),
        )
        .await
        .unwrap_err();
    assert!(is_code(&err, AuthErrorCode::AccountConflict), "{err}");

    let verified = ExternalUserinfo {
        email: Some(ProviderEmail {
            address: "ui@example.com".into(),
            verified: true,
        }),
        name: None,
    };
    let (_user, is_new) = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &bare,
            Some(&verified),
        )
        .await
        .unwrap();
    assert!(!is_new);
}

#[tokio::test]
async fn userinfo_is_only_fetched_when_it_could_matter() {
    let (state, _tmp) = test_app_state().await;
    let on = service(&state, true, true);
    let off = service(&state, false, true);

    assert!(!on.needs_userinfo(&login(ISSUER, "s", Some(("a@example.com", true)))));
    assert!(on.needs_userinfo(&login(ISSUER, "s", Some(("a@example.com", false)))));
    assert!(on.needs_userinfo(&login(ISSUER, "s", None)));
    assert!(!off.needs_userinfo(&login(ISSUER, "s", None)));
}

#[tokio::test]
async fn email_matching_is_off_by_default_and_never_duplicates_an_account() {
    let (state, _tmp) = test_app_state().await;
    register_user(&state, "local", "local@example.com", "password123").await;
    assert!(!state.config.sso.signups_match_email);
    let svc = service(&state, false, true);

    let err = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "sub-x", Some(("local@example.com", true))),
            None,
        )
        .await
        .unwrap_err();
    assert!(is_code(&err, AuthErrorCode::AccountConflict), "{err}");
}

#[tokio::test]
async fn an_unverified_email_with_no_account_still_signs_up_in_permissive_mode() {
    let (state, _tmp) = test_app_state().await;
    let svc = service(&state, true, true);

    let (user, is_new) = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "fresh", Some(("fresh@example.com", false))),
            None,
        )
        .await
        .unwrap();
    assert!(is_new);
    assert_eq!(user.email, "fresh@example.com");
}

#[tokio::test]
async fn strict_mode_refuses_an_email_the_provider_did_not_verify() {
    let (state, _tmp) = test_app_state().await;
    let strict = service(&state, true, false);

    let err = strict
        .check_email_claim(&login(ISSUER, "s", Some(("a@example.com", false))))
        .unwrap_err();
    assert!(is_code(&err, AuthErrorCode::EmailNotVerified), "{err}");
    assert!(
        strict
            .check_email_claim(&login(ISSUER, "s", Some(("a@example.com", true))))
            .is_ok()
    );
    // No email to vouch for: nothing to refuse.
    assert!(strict.check_email_claim(&login(ISSUER, "s", None)).is_ok());

    let permissive = service(&state, true, true);
    assert!(
        permissive
            .check_email_claim(&login(ISSUER, "s", Some(("a@example.com", false))))
            .is_ok()
    );
}

#[tokio::test]
async fn an_account_bound_to_another_subject_cannot_be_taken_over_by_email() {
    let (state, _tmp) = test_app_state().await;
    let svc = service(&state, true, true);

    // The first person to sign in with this address gets the account...
    svc.sign_up(
        &state.user_service,
        &state.token_service,
        &login(ISSUER, "first", Some(("shared@example.com", false))),
        None,
    )
    .await
    .unwrap();

    // ...so a later, different subject asserting the same verified address
    // does not get linked into it.
    let err = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "second", Some(("shared@example.com", true))),
            None,
        )
        .await
        .unwrap_err();
    assert!(is_code(&err, AuthErrorCode::AccountConflict), "{err}");
}

#[tokio::test]
async fn a_subject_only_means_something_within_its_issuer() {
    let (state, _tmp) = test_app_state().await;
    let svc = service(&state, true, true);

    let (owner, _) = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(ISSUER, "same-sub", Some(("owner@example.com", true))),
            None,
        )
        .await
        .unwrap();

    let known = svc
        .known_user(&state.user_service, &login(ISSUER, "same-sub", None))
        .await
        .unwrap();
    assert_eq!(known.map(|u| u.id), Some(owner.id.clone()));

    // The same subject from another issuer is somebody else.
    let other = svc
        .known_user(
            &state.user_service,
            &login("https://other-idp.example.test", "same-sub", None),
        )
        .await
        .unwrap();
    assert!(other.is_none());

    // And can hold its own identity without colliding on the unique index.
    let (stranger, is_new) = svc
        .sign_up(
            &state.user_service,
            &state.token_service,
            &login(
                "https://other-idp.example.test",
                "same-sub",
                Some(("stranger@example.com", true)),
            ),
            None,
        )
        .await
        .unwrap();
    assert!(is_new);
    assert_ne!(stranger.id, owner.id);
}

#[tokio::test]
async fn a_link_recorded_before_issuers_were_tracked_is_adopted_on_next_sign_in() {
    let (state, _tmp) = test_app_state().await;
    let (_t, user_id) = register_user(&state, "legacy", "legacy@example.com", "password123").await;
    let now = chrono::Utc::now();
    use frona::core::repository::Repository;
    repo(&state)
        .create(&OAuthIdentity {
            id: frona::core::repository::new_id(),
            user_id: user_id.clone(),
            issuer: None,
            external_sub: "old-sub".into(),
            external_email: None,
            external_name: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();

    let svc = service(&state, false, true);
    let user = svc
        .known_user(&state.user_service, &login(ISSUER, "old-sub", None))
        .await
        .unwrap()
        .expect("legacy link still works");
    assert_eq!(user.id, user_id);

    let stored = repo(&state)
        .find_identity(ISSUER, "old-sub")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.issuer.as_deref(), Some(ISSUER));
}
