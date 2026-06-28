# Web Push Notifications Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Enable browser/OS-native push notifications for frona on desktop AND mobile (Android Chrome, iOS Safari PWA), including background delivery when the tab/browser is closed.

**Architecture:** Add a Service Worker to the Next.js frontend that handles `push` and `notificationclick` events. The backend gains a VAPID key pair (config-based), a `push_subscription` SurrealDB table, and REST endpoints for subscription management. When `NotificationService.create()` fires, it sends Web Push messages to all of the user's stored push subscriptions in parallel with the existing SSE broadcast. The `web-push` Rust crate handles VAPID JWT signing + RFC 8291 content encryption.

**Tech Stack:** Rust (`web-push` crate, `base64`), SurrealDB, Next.js 16 (Service Worker via `public/sw.js`), React 19, Web Push API, VAPID.

---

## Current Context

### What exists today
- `NotificationService` (`crates/frona-server/src/notification/service.rs`) creates `Notification` records in SurrealDB.
- `BroadcastService::send_notification()` (`crates/frona-server/src/chat/broadcast.rs:445`) dispatches SSE events (`NewNotification` variant → `event: notification` SSE).
- Frontend `session-context.ts:151-153` receives the SSE `notification` event and calls `addNotification()`.
- `notification-context.tsx:50-54` prepends to React state and bumps `unreadCount` — but never calls the browser Notification API.
- Three call sites create notifications:
  1. `tool/send_message.rs:105-121` — agent sends a message → notification
  2. `tool/manage_app.rs:297-321` — app lifecycle events (crash, start, etc.)
  3. `core/supervisor.rs:189-209` — supervisor restore/crash events (`send_notification` helper)

### What's missing
- No Service Worker registered in the frontend.
- No VAPID key pair or config.
- No `push_subscription` DB table / model / repository.
- No REST endpoints for subscription management.
- No Web Push send hook in the notification pipeline.
- No "Enable notifications" UI.

### Assumptions
- The deploy server (`myfrona.morganpercy.com`) is HTTPS — required for Service Workers and Push API.
- VAPID keys are generated once with `npx web-push generate-vapid-keys` and stored in frona's config (env vars or YAML).
- One user can have multiple push subscriptions (phone + laptop + desktop).
- If VAPID keys are not configured, push is silently disabled (graceful degradation).

---

## Phase 1: Backend — Push Subscription Storage

### Task 1: Create `PushSubscription` model

**Objective:** Define the SurrealDB entity for storing per-user push subscriptions.

**Files:**
- Create: `crates/frona-server/src/notification/push_model.rs`
- Modify: `crates/frona-server/src/notification/mod.rs`

**Step 1: Create the model file**

```rust
// crates/frona-server/src/notification/push_model.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::SurrealValue;
use crate::Entity;

/// A stored Web Push subscription for a user.
/// One user can have multiple (phone, laptop, desktop).
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Entity)]
#[surreal(crate = "surrealdb::types")]
#[entity(table = "push_subscription")]
pub struct PushSubscription {
    pub id: String,
    pub user_id: String,
    /// The push endpoint URL (unique per browser/device).
    pub endpoint: String,
    /// Expiration time in milliseconds since epoch, or None if no expiration.
    pub expiration_time: Option<i64>,
    /// The P-256 ECDSA public key (base64url-encoded).
    pub p256dh_key: String,
    /// The authentication secret (base64url-encoded).
    pub auth_secret: String,
    pub created_at: DateTime<Utc>,
}
```

**Step 2: Add module to `mod.rs`**

```rust
// crates/frona-server/src/notification/mod.rs
pub mod models;
pub mod push_model;
pub mod repository;
pub mod service;
```

**Step 3: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add crates/frona-server/src/notification/push_model.rs crates/frona-server/src/notification/mod.rs
git commit -m "feat(push): add PushSubscription model"
```

---

### Task 2: Create `PushSubscriptionRepository` trait + SurrealDB impl

**Objective:** Repository for CRUD on push subscriptions, keyed by user_id and endpoint.

**Files:**
- Create: `crates/frona-server/src/notification/push_repository.rs`
- Create: `crates/frona-server/src/db/repo/push_subscriptions.rs`
- Modify: `crates/frona-server/src/db/repo/mod.rs`
- Modify: `crates/frona-server/src/notification/mod.rs`

**Step 1: Create the repository trait**

```rust
// crates/frona-server/src/notification/push_repository.rs
use async_trait::async_trait;
use crate::core::error::AppError;
use crate::core::repository::Repository;
use super::push_model::PushSubscription;

#[async_trait]
pub trait PushSubscriptionRepository: Repository<PushSubscription> {
    /// Find all push subscriptions for a user.
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<PushSubscription>, AppError>;
    /// Find a subscription by its endpoint URL (for dedup / delete).
    async fn find_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<Option<PushSubscription>, AppError>;
    /// Delete a subscription by endpoint URL.
    async fn delete_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<(), AppError>;
}
```

**Step 2: Create the SurrealDB impl**

```rust
// crates/frona-server/src/db/repo/push_subscriptions.rs
use async_trait::async_trait;
use crate::core::error::AppError;
use crate::notification::push_model::PushSubscription;
use crate::notification::push_repository::PushSubscriptionRepository;
use super::generic::SurrealRepo;

pub type SurrealPushSubscriptionRepo = SurrealRepo<PushSubscription>;

const SELECT_CLAUSE: &str = "SELECT *, meta::id(id) as id";

#[async_trait]
impl PushSubscriptionRepository for SurrealRepo<PushSubscription> {
    async fn find_by_user_id(&self, user_id: &str) -> Result<Vec<PushSubscription>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM push_subscription WHERE user_id = $user_id ORDER BY created_at DESC"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let subs: Vec<PushSubscription> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(subs)
    }

    async fn find_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<Option<PushSubscription>, AppError> {
        let query = format!(
            "{SELECT_CLAUSE} FROM push_subscription WHERE user_id = $user_id AND endpoint = $endpoint LIMIT 1"
        );
        let mut result = self
            .db()
            .query(&query)
            .bind(("user_id", user_id.to_string()))
            .bind(("endpoint", endpoint.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let subs: Vec<PushSubscription> = result
            .take(0)
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(subs.into_iter().next())
    }

    async fn delete_by_endpoint(&self, user_id: &str, endpoint: &str) -> Result<(), AppError> {
        self.db()
            .query("DELETE FROM push_subscription WHERE user_id = $user_id AND endpoint = $endpoint")
            .bind(("user_id", user_id.to_string()))
            .bind(("endpoint", endpoint.to_string()))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}
```

**Step 3: Add module to `db/repo/mod.rs`**

Add `pub mod push_subscriptions;` to `crates/frona-server/src/db/repo/mod.rs`.

**Step 4: Add module to `notification/mod.rs`**

Add `pub mod push_repository;` to `crates/frona-server/src/notification/mod.rs`.

**Step 5: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles with no errors

**Step 6: Commit**

```bash
git add crates/frona-server/src/notification/push_repository.rs crates/frona-server/src/db/repo/push_subscriptions.rs crates/frona-server/src/db/repo/mod.rs crates/frona-server/src/notification/mod.rs
git commit -m "feat(push): add PushSubscriptionRepository trait + SurrealDB impl"
```

---

### Task 3: Add VAPID config struct

**Objective:** Add `PushConfig` to the server config with VAPID public/private keys and subject.

**Files:**
- Modify: `crates/frona-server/src/core/config.rs` (add `PushConfig` struct + `pub push: PushConfig` field to `Config`)
- Modify: `crates/frona-server/src/lib.rs` or wherever `Config` is documented (env vars)

**Step 1: Add `PushConfig` struct**

Add this after the `ShareConfig` impl block (around line 390 in `config.rs`):

```rust
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct PushConfig {
    #[schemars(description = "VAPID public key (base64url-encoded, uncompressed P-256). Required for Web Push.")]
    pub vapid_public_key: Option<String>,
    #[schemars(description = "VAPID private key (base64url-encoded, uncompressed P-256). Required for Web Push.")]
    pub vapid_private_key: Option<String>,
    #[schemars(description = "VAPID subject — a mailto: URL or the site's HTTPS URL.")]
    pub subject: String,
}

impl Default for PushConfig {
    fn default() -> Self {
        Self {
            vapid_public_key: None,
            vapid_private_key: None,
            subject: "mailto:noreply@frona.local".into(),
        }
    }
}
```

**Step 2: Add `push` field to `Config` struct**

In the `Config` struct (around line 944), add after `pub share: ShareConfig`:

```rust
    #[serde(default)]
    pub push: PushConfig,
```

**Step 3: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add crates/frona-server/src/core/config.rs
git commit -m "feat(push): add PushConfig with VAPID key fields"
```

---

### Task 4: Add `web-push` crate dependency

**Objective:** Add the `web-push` Rust crate to `frona-server`'s `Cargo.toml`.

**Files:**
- Modify: `crates/frona-server/Cargo.toml`

**Step 1: Add dependency**

Add to `[dependencies]` in `crates/frona-server/Cargo.toml`:

```toml
web-push = "0.10"
```

> **Pitfall:** The `web-push` crate API has changed across versions. The implementer should check https://docs.rs/web-push for the pinned version's API. Key types: `WebPushClient`, `SubscriptionInfo`, `PartialVapidSignature`, `SubscriptionEndpoint`, `WebPushMessage`, `WebPushError`.

**Step 2: Verify it compiles (fetch + build)**

Run: `cargo check -p frona-server`
Expected: compiles (may take a while first time — fetches the crate)

**Step 3: Commit**

```bash
git add crates/frona-server/Cargo.toml
git commit -m "feat(push): add web-push crate dependency"
```

---

### Task 5: REST endpoints for push subscription management

**Objective:** Add REST endpoints: `GET /api/push/vapid-public-key`, `POST /api/push/subscribe`, `POST /api/push/unsubscribe`.

**Files:**
- Create: `crates/frona-server/src/api/routes/push.rs`
- Modify: `crates/frona-server/src/api/routes/mod.rs` (add `pub mod push;`)
- Modify: `crates/frona-server/src/main.rs` (add `.merge(routes::push::router())`)
- Modify: `crates/frona-server/src/core/state.rs` (add `push_subscription_repo` to `AppState`)

**Step 1: Add `push_subscription_repo` to `AppState`**

In `crates/frona-server/src/core/state.rs`:

In the `AppState` struct (around line 113), add:
```rust
    pub push_subscription_repo: SurrealPushSubscriptionRepo,
```

In `AppState::new()`, after `notification_service` is created (around line 465), add:
```rust
    push_subscription_repo: SurrealRepo::new(db.clone()),
```

Add imports at the top:
```rust
use crate::db::repo::push_subscriptions::SurrealPushSubscriptionRepo;
use crate::notification::push_model::PushSubscription;
```

**Step 2: Create the route handler**

```rust
// crates/frona-server/src/api/routes/push.rs
use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::core::state::AppState;
use crate::core::repository::Repository;
use crate::notification::push_model::PushSubscription;

use super::super::error::ApiError;
use super::super::middleware::auth::AuthUser;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/push/vapid-public-key", get(get_vapid_public_key))
        .route("/api/push/subscribe", post(subscribe))
        .route("/api/push/unsubscribe", post(unsubscribe))
}

#[derive(Serialize)]
struct VapidPublicKeyResponse {
    public_key: Option<String>,
}

async fn get_vapid_public_key(
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<VapidPublicKeyResponse>, ApiError> {
    Ok(Json(VapidPublicKeyResponse {
        public_key: state.config.push.vapid_public_key.clone(),
    }))
}

#[derive(Deserialize)]
struct SubscribeRequest {
    endpoint: String,
    expiration_time: Option<i64>,
    keys: SubscriptionKeys,
}

#[derive(Deserialize)]
struct SubscriptionKeys {
    p256dh: String,
    auth: String,
}

async fn subscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<SubscribeRequest>,
) -> Result<(), ApiError> {
    let sub = PushSubscription {
        id: crate::core::repository::new_id(),
        user_id: auth.user_id.clone(),
        endpoint: req.endpoint.clone(),
        expiration_time: req.expiration_time,
        p256dh_key: req.keys.p256dh,
        auth_secret: req.keys.auth,
        created_at: chrono::Utc::now(),
    };

    // Dedup: if a subscription with this endpoint already exists, update it.
    if let Some(existing) = state
        .push_subscription_repo
        .find_by_endpoint(&auth.user_id, &req.endpoint)
        .await?
    {
        state
            .push_subscription_repo
            .delete_by_endpoint(&auth.user_id, &req.endpoint)
            .await?;
    }

    state.push_subscription_repo.create(&sub).await?;
    Ok(())
}

#[derive(Deserialize)]
struct UnsubscribeRequest {
    endpoint: String,
}

async fn unsubscribe(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(req): Json<UnsubscribeRequest>,
) -> Result<(), ApiError> {
    state
        .push_subscription_repo
        .delete_by_endpoint(&auth.user_id, &req.endpoint)
        .await?;
    Ok(())
}
```

**Step 3: Register module in `routes/mod.rs`**

Add `pub mod push;` to `crates/frona-server/src/api/routes/mod.rs`.

**Step 4: Register router in `main.rs`**

In `crates/frona-server/src/main.rs`, after `.merge(routes::notifications::router())` (line 257), add:
```rust
        .merge(routes::push::router())
```

**Step 5: Write a test**

Create `crates/frona-server/tests/api/push.rs`:

```rust
use frona::notification::push_model::PushSubscription;
use frona::core::repository::Repository;

#[tokio::test]
async fn subscribe_and_unsubscribe_flow() {
    let (state, _tmp) = frona::test_utils::test_app_state().await;
    let user_id = "test-user";

    let sub = PushSubscription {
        id: "sub-1".into(),
        user_id: user_id.into(),
        endpoint: "https://fcm.googleapis.com/fcm/send/abc123".into(),
        expiration_time: None,
        p256dh_key: "test-p256dh".into(),
        auth_secret: "test-auth".into(),
        created_at: chrono::Utc::now(),
    };

    state.push_subscription_repo.create(&sub).await.unwrap();
    let found = state
        .push_subscription_repo
        .find_by_user_id(user_id)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].endpoint, sub.endpoint);

    state
        .push_subscription_repo
        .delete_by_endpoint(user_id, &sub.endpoint)
        .await
        .unwrap();
    let found = state
        .push_subscription_repo
        .find_by_user_id(user_id)
        .await
        .unwrap();
    assert!(found.is_empty());
}
```

> **Note:** The implementer should check how existing tests in `tests/api/` set up `test_app_state()` — the pattern is visible in `tests/api/notifications.rs`.

**Step 6: Run the test**

Run: `cargo test -p frona-server --test push subscribe_and_unsubscribe`
Expected: PASS

**Step 7: Commit**

```bash
git add crates/frona-server/src/api/routes/push.rs crates/frona-server/src/api/routes/mod.rs crates/frona-server/src/main.rs crates/frona-server/src/core/state.rs crates/frona-server/tests/api/push.rs
git commit -m "feat(push): add REST endpoints for subscription management"
```

---

## Phase 2: Push Send Hook

### Task 6: Create `PushSender` module

**Objective:** A module that takes a `Notification` + user_id, looks up the user's push subscriptions, and sends Web Push messages to each endpoint using VAPID signing.

**Files:**
- Create: `crates/frona-server/src/notification/push_sender.rs`
- Modify: `crates/frona-server/src/notification/mod.rs`

**Step 1: Create the `PushSender` struct**

```rust
// crates/frona-server/src/notification/push_sender.rs
use std::sync::Arc;
use base64::Engine;
use web_push::{
    WebPushClient, WebPushError, SubscriptionInfo, SubscriptionEndpoint,
    PartialVapidSignature, VapidSignature, WebPushMessageBuilder,
};
use crate::core::config::PushConfig;
use crate::notification::models::Notification;
use crate::notification::push_repository::PushSubscriptionRepository;
use crate::core::error::AppError;

pub struct PushSender {
    client: WebPushClient,
    vapid_signature: VapidSignature,
    repo: Arc<dyn PushSubscriptionRepository>,
}

impl PushSender {
    /// Returns `Some(PushSender)` if VAPID keys are configured, else `None`.
    pub fn new(
        config: &PushConfig,
        repo: Arc<dyn PushSubscriptionRepository>,
    ) -> Result<Option<Self>, AppError> {
        let public_key = match &config.vapid_public_key {
            Some(k) => k,
            None => return Ok(None),
        };
        let private_key = match &config.vapid_private_key {
            Some(k) => k,
            None => return Ok(None),
        };

        let public_key_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(public_key)
            .map_err(|e| AppError::Internal(format!("Invalid VAPID public key: {e}")))?;
        let private_key_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(private_key)
            .map_err(|e| AppError::Internal(format!("Invalid VAPID private key: {e}")))?;

        let partial = PartialVapidSignature::from_pem(
            &public_key_bytes,
            &private_key_bytes,
            &config.subject,
        ).map_err(|e| AppError::Internal(format!("VAPID key parse error: {e}")))?;

        let client = WebPushClient::new();
        let vapid_signature = partial.into();

        Ok(Some(Self {
            client,
            vapid_signature,
            repo,
        }))
    }

    /// Send a push notification to all of the user's subscriptions.
    /// Fire-and-forget — logs errors but doesn't fail the caller.
    pub async fn send_to_user(&self, user_id: &str, notification: &Notification) {
        let subs = match self.repo.find_by_user_id(user_id).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, user_id, "Failed to fetch push subscriptions");
                return;
            }
        };

        if subs.is_empty() {
            return;
        }

        let payload = serde_json::json!({
            "id": notification.id,
            "title": notification.title,
            "body": notification.body,
            "level": format!("{:?}", notification.level).to_lowercase(),
            "data": notification.data,
            "url": self.deep_link(&notification.data),
        });

        let payload_bytes = serde_json::to_vec(&payload).unwrap_or_default();
        let ttl = chrono::Duration::hours(24).num_seconds() as u32;

        for sub in subs {
            let subscription_info = SubscriptionInfo::new(
                &sub.endpoint,
                &sub.p256dh_key,
                &sub.auth_secret,
            );

            let mut builder = WebPushMessageBuilder::new(&subscription_info);
            builder.set_payload(web_push::WebPushPayloadContentType::Json, &payload_bytes);
            builder.set_ttl(ttl);

            match self.client.send(builder.build(&self.vapid_signature).unwrap()).await {
                Ok(_) => {}
                Err(WebPushError::EndpointNotValid | WebPushError::EndpointNotFound) => {
                    // Subscription expired — clean it up.
                    tracing::info!(endpoint = %sub.endpoint, "Removing expired push subscription");
                    let _ = self.repo.delete_by_endpoint(user_id, &sub.endpoint).await;
                }
                Err(e) => {
                    tracing::warn!(error = %e, endpoint = %sub.endpoint, "Push send failed");
                }
            }
        }
    }

    /// Map notification data to a deep-link URL for click-through.
    fn deep_link(&self, data: &crate::notification::models::NotificationData) -> String {
        use crate::notification::models::NotificationData;
        match data {
            NotificationData::Agent { chat_id, .. } => format!("/chat?id={}", chat_id),
            NotificationData::App { app_handle, .. } => format!("/apps/{}", app_handle),
            NotificationData::Task { task_id } => format!("/?task={}", task_id),
            _ => "/".to_string(),
        }
    }
}
```

> **Pitfall:** The `web-push` crate API (`PartialVapidSignature`, `VapidSignature`, `WebPushMessageBuilder`) may differ between versions. The implementer MUST check https://docs.rs/web-push for the exact API of the pinned version (0.10). Key construction may need `PartialVapidSignature::from_pem()` or `from_base64_no_padding()` depending on key format. Adjust accordingly.

**Step 2: Add module to `mod.rs`**

Add `pub mod push_sender;` to `crates/frona-server/src/notification/mod.rs`.

**Step 3: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles (fix web-push API calls as needed per docs.rs)

**Step 4: Commit**

```bash
git add crates/frona-server/src/notification/push_sender.rs crates/frona-server/src/notification/mod.rs
git commit -m "feat(push): add PushSender module with VAPID signing"
```

---

### Task 7: Wire `PushSender` into `AppState` and `NotificationService`

**Objective:** Add `push_sender: Option<PushSender>` to `AppState`. Add a `create_and_notify()` method to `NotificationService` that creates the notification, sends SSE, and fires Web Push in parallel.

**Files:**
- Modify: `crates/frona-server/src/core/state.rs`
- Modify: `crates/frona-server/src/notification/service.rs`

**Step 1: Add `push_sender` to `AppState`**

In `crates/frona-server/src/core/state.rs`:

In the `AppState` struct, add:
```rust
    pub push_sender: Option<Arc<crate::notification::push_sender::PushSender>>,
```

In `AppState::new()`, after `notification_service` is created, add:
```rust
        let push_subscription_repo: Arc<dyn crate::notification::push_repository::PushSubscriptionRepository> =
            Arc::new(crate::db::repo::push_subscriptions::SurrealPushSubscriptionRepo::new(db.clone()));
        let push_sender = crate::notification::push_sender::PushSender::new(
            &config.push,
            push_subscription_repo.clone(),
        )
        .ok()
        .flatten()
        .map(Arc::new);
```

In the `Self { ... }` block, add:
```rust
            push_sender,
```

Also add the `push_subscription_repo` to the struct if not already there from Task 5, or reuse the one created here.

**Step 2: Add `create_and_notify()` to `NotificationService`**

```rust
// In crates/frona-server/src/notification/service.rs, add:

use crate::chat::broadcast::BroadcastService;
use crate::notification::push_sender::PushSender;
use std::sync::Arc;

impl NotificationService {
    /// Creates a notification, broadcasts it via SSE, and fires Web Push (if configured).
    /// Web Push is fire-and-forget via `tokio::spawn` — does not block or fail this call.
    pub async fn create_and_notify(
        &self,
        user_id: &str,
        data: NotificationData,
        level: NotificationLevel,
        title: String,
        body: String,
        broadcast_service: &BroadcastService,
        push_sender: &Option<Arc<PushSender>>,
    ) -> Result<Notification, AppError> {
        let notification = self.create(user_id, data, level, title, body).await?;

        // SSE broadcast (existing path)
        broadcast_service.send_notification(user_id, notification.clone());

        // Web Push (new path) — fire and forget
        if let Some(sender) = push_sender {
            let sender = sender.clone();
            let uid = user_id.to_string();
            let notif = notification.clone();
            tokio::spawn(async move {
                sender.send_to_user(&uid, &notif).await;
            });
        }

        Ok(notification)
    }
}
```

**Step 3: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add crates/frona-server/src/core/state.rs crates/frona-server/src/notification/service.rs
git commit -m "feat(push): wire PushSender into AppState and NotificationService"
```

---

### Task 8: Update call sites to use `create_and_notify()`

**Objective:** Replace the 3 existing `notification_service.create()` + `broadcast_service.send_notification()` pairs with the new `create_and_notify()` method.

**Files:**
- Modify: `crates/frona-server/src/tool/send_message.rs` (lines 105-121)
- Modify: `crates/frona-server/src/tool/manage_app.rs` (lines 305-321)
- Modify: `crates/frona-server/src/core/supervisor.rs` (lines 189-209)

**Step 1: Update `send_message.rs`**

Replace lines 105-121 in `crates/frona-server/src/tool/send_message.rs`:

```rust
        // Before:
        // if let Ok(notification) = self.notification_service.create(...).await {
        //     self.broadcast_service.send_notification(&ctx.user.id, notification);
        // }

        // After:
        let push_sender = &ctx.state.push_sender; // or pass through InferenceContext
        let _ = self
            .notification_service
            .create_and_notify(
                &ctx.user.id,
                NotificationData::Agent {
                    agent_id: ctx.agent.id.clone(),
                    chat_id: resolved_chat.id.clone(),
                },
                NotificationLevel::Info,
                ctx.agent.name.clone(),
                truncated_body,
                &self.broadcast_service,
                push_sender,
            )
            .await;
```

> **Pitfall:** `InferenceContext` may not have direct access to `AppState` or `push_sender`. The implementer needs to check how `SendMessageTool` gets its dependencies (it's constructed in `harness.rs` or wherever tools are wired). The `PushSender` may need to be passed into the tool's constructor alongside `notification_service` and `broadcast_service`. Check the tool construction site and add `push_sender` as a field on `SendMessageTool`.

**Step 2: Update `manage_app.rs`**

Replace the `emit_notification` method (lines 297-321) in `crates/frona-server/src/tool/manage_app.rs`:

```rust
    async fn emit_notification(
        &self,
        ctx: &InferenceContext,
        app_handle: &str,
        action: &str,
        level: NotificationLevel,
        title: &str,
    ) {
        let _ = self
            .notification_service
            .create_and_notify(
                &ctx.user.id,
                NotificationData::App {
                    app_handle: app_handle.to_string(),
                    action: action.to_string(),
                },
                level,
                title.to_string(),
                String::new(),
                &self.broadcast_service,
                &self.push_sender, // new field
            )
            .await;
    }
```

Same pitfall — add `push_sender` as a field on `ManageAppTool`.

**Step 3: Update `supervisor.rs`**

Replace the `send_notification` helper function (lines 189-209) in `crates/frona-server/src/core/supervisor.rs`:

```rust
async fn send_notification<S: Supervisor>(
    supervisor: &Arc<S>,
    notification_service: &NotificationService,
    broadcast_service: &BroadcastService,
    push_sender: &Option<Arc<PushSender>>,
    id: &str,
    action: &str,
    level: NotificationLevel,
    title: &str,
    body: &str,
) {
    let Ok(user_id) = supervisor.owner_of(id).await else {
        return;
    };
    let data = supervisor.notification_data(id, action).await;
    let _ = notification_service
        .create_and_notify(
            &user_id,
            data,
            level,
            title.to_string(),
            body.to_string(),
            broadcast_service,
            push_sender,
        )
        .await;
}
```

Then update the two call sites of `send_notification()` (lines 103 and 142) to pass `push_sender` through. The supervisor functions receive `push_sender` as a new parameter — check the caller chain to thread it through.

**Step 4: Verify it compiles**

Run: `cargo check -p frona-server`
Expected: compiles with no errors

**Step 5: Run existing tests**

Run: `cargo test -p frona-server --test notifications`
Expected: PASS (no regression)

**Step 6: Commit**

```bash
git add crates/frona-server/src/tool/send_message.rs crates/frona-server/src/tool/manage_app.rs crates/frona-server/src/core/supervisor.rs
git commit -m "feat(push): update notification call sites to use create_and_notify"
```

---

## Phase 3: Frontend — Service Worker + Push Subscription

### Task 9: Create the Service Worker

**Objective:** Create `public/sw.js` that handles `push` and `notificationclick` events.

**Files:**
- Create: `web/public/sw.js`

**Step 1: Create the service worker file**

```javascript
// web/public/sw.js

// Handle push events — show a notification.
self.addEventListener("push", (event) => {
  let data;
  try {
    data = event.data ? event.data.json() : {};
  } catch {
    data = { title: "Frona", body: event.data ? event.data.text() : "" };
  }

  const title = data.title || "Frona";
  const options = {
    body: data.body || "",
    icon: "/icon-192.png",
    badge: "/badge-72.png",
    tag: data.id || undefined,
    data: {
      url: data.url || "/",
      id: data.id,
    },
  };

  event.waitUntil(self.registration.showNotification(title, options));
});

// Handle notification click — focus or open the app at the deep-link URL.
self.addEventListener("notificationclick", (event) => {
  event.notification.close();

  const targetUrl = event.notification.data?.url || "/";

  event.waitUntil(
    (async () => {
      const allClients = await clients.matchAll({
        type: "window",
        includeUncontrolled: true,
      });

      // Focus an existing tab if one is open.
      for (const client of allClients) {
        if (client.url.includes(self.location.origin)) {
          if ("focus" in client) {
            await client.focus();
            if ("navigate" in client) {
              await client.navigate(targetUrl);
            }
          }
          return;
        }
      }

      // No existing tab — open a new one.
      if (clients.openWindow) {
        await clients.openWindow(targetUrl);
      }
    })(),
  );
});

// Handle subscription expiration / pushsubscriptionchange.
self.addEventListener("pushsubscriptionchange", (event) => {
  event.waitUntil(
    (async () => {
      const registration = await self.registration;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: await getVapidKey(),
      });
      await sendSubscriptionToServer(subscription);
    })(),
  );
});

async function getVapidKey() {
  const res = await fetch("/api/push/vapid-public-key", {
    credentials: "include",
  });
  const data = await res.json();
  return urlBase64ToUint8Array(data.public_key);
}

async function sendSubscriptionToServer(subscription) {
  await fetch("/api/push/subscribe", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    credentials: "include",
    body: JSON.stringify(subscription),
  });
}

function urlBase64ToUint8Array(base64String) {
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const rawData = atob(base64);
  const outputArray = new Uint8Array(rawData.length);
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i);
  }
  return outputArray;
}
```

> **Pitfall:** Next.js 16 serves files from `public/` at the root path. The SW at `public/sw.js` will be accessible at `/sw.js`. Ensure the SW scope covers the entire app (default for `/sw.js`).

> **Note:** The `icon-192.png` and `badge-72.png` are optional — if they don't exist, the notification still works but without a custom icon. The implementer can add placeholder icons or remove the icon/badge lines.

**Step 2: Commit**

```bash
git add web/public/sw.js
git commit -m "feat(push): add service worker with push + notificationclick handlers"
```

---

### Task 10: Register the Service Worker on app load

**Objective:** Register `/sw.js` when the app loads (client-side only, after auth).

**Files:**
- Create: `web/src/lib/sw-register.ts`
- Modify: `web/src/app/(main)/layout.tsx`

**Step 1: Create the registration helper**

```typescript
// web/src/lib/sw-register.ts

export async function registerServiceWorker(): Promise<ServiceWorkerRegistration | null> {
  if (typeof window === "undefined") return null;
  if (!("serviceWorker" in navigator)) return null;

  try {
    const registration = await navigator.serviceWorker.register("/sw.js", {
      scope: "/",
    });
    console.log("[sw] Service Worker registered:", registration.scope);
    return registration;
  } catch (err) {
    console.error("[sw] Service Worker registration failed:", err);
    return null;
  }
}
```

**Step 2: Register in the main layout**

In `web/src/app/(main)/layout.tsx`, add a `useEffect` to register the SW after the component mounts:

```tsx
"use client";

import { Suspense } from "react";
import { useEffect } from "react";
import { AppGate } from "@/components/app-gate";
import { NavigationProvider } from "@/lib/navigation-context";
import { NotificationProvider } from "@/lib/notification-context";
import { SessionProvider } from "@/lib/session-context";
import { TopBar } from "@/components/layout/top-bar";
import { registerServiceWorker } from "@/lib/sw-register";

export default function MainLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  useEffect(() => {
    registerServiceWorker();
  }, []);

  return (
    <NavigationProvider>
      <AppGate>
        <NotificationProvider>
          <Suspense>
            <SessionProvider>
              <div className="flex flex-col h-screen">
                <TopBar />
                <div className="flex-1 overflow-hidden">
                  {children}
                </div>
              </div>
            </SessionProvider>
          </Suspense>
        </NotificationProvider>
      </AppGate>
    </NavigationProvider>
  );
}
```

**Step 3: Verify the frontend builds**

Run: `cd web && npm run build`
Expected: builds with no errors

**Step 4: Commit**

```bash
git add web/src/lib/sw-register.ts web/src/app/(main)/layout.tsx
git commit -m "feat(push): register service worker on app load"
```

---

### Task 11: Create `usePushNotifications` React hook

**Objective:** A hook that requests notification permission, subscribes to push, and posts the subscription to the backend.

**Files:**
- Create: `web/src/lib/use-push-notifications.ts`

**Step 1: Create the hook**

```typescript
// web/src/lib/use-push-notifications.ts
"use client";

import { useState, useEffect, useCallback } from "react";
import { api } from "./api-client";

type PermissionState = "default" | "granted" | "denied" | "unsupported";

export function usePushNotifications() {
  const [permission, setPermission] = useState<PermissionState>("default");
  const [subscribed, setSubscribed] = useState(false);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (!("Notification" in window) || !("serviceWorker" in navigator)) {
      setPermission("unsupported");
      return;
    }
    setPermission(Notification.permission as PermissionState);

    // Check if already subscribed.
    navigator.serviceWorker.ready
      .then((reg) => reg.pushManager.getSubscription())
      .then((sub) => {
        if (sub) setSubscribed(true);
      })
      .catch(() => {});
  }, []);

  const enable = useCallback(async () => {
    if (permission === "unsupported") return;
    setLoading(true);
    try {
      // 1. Request notification permission.
      const result = await Notification.requestPermission();
      setPermission(result as PermissionState);
      if (result !== "granted") return;

      // 2. Fetch VAPID public key from backend.
      const { public_key } = await api.get<{ public_key: string }>(
        "/api/push/vapid-public-key",
      );
      if (!public_key) {
        console.error("[push] VAPID public key not configured on server");
        return;
      }

      // 3. Subscribe via the service worker.
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: urlBase64ToUint8Array(public_key),
      });

      // 4. POST subscription to backend.
      await api.post("/api/push/subscribe", subscription.toJSON());
      setSubscribed(true);
    } catch (err) {
      console.error("[push] Failed to enable notifications:", err);
    } finally {
      setLoading(false);
    }
  }, [permission]);

  const disable = useCallback(async () => {
    setLoading(true);
    try {
      const registration = await navigator.serviceWorker.ready;
      const subscription = await registration.pushManager.getSubscription();
      if (subscription) {
        await api.post("/api/push/unsubscribe", {
          endpoint: subscription.endpoint,
        });
        await subscription.unsubscribe();
      }
      setSubscribed(false);
    } catch (err) {
      console.error("[push] Failed to disable notifications:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  return { permission, subscribed, loading, enable, disable };
}

function urlBase64ToUint8Array(base64String: string): Uint8Array {
  const padding = "=".repeat((4 - (base64String.length % 4)) % 4);
  const base64 = (base64String + padding).replace(/-/g, "+").replace(/_/g, "/");
  const rawData = atob(base64);
  const outputArray = new Uint8Array(rawData.length);
  for (let i = 0; i < rawData.length; ++i) {
    outputArray[i] = rawData.charCodeAt(i);
  }
  return outputArray;
}
```

**Step 2: Verify the frontend builds**

Run: `cd web && npm run build`
Expected: builds with no errors

**Step 3: Commit**

```bash
git add web/src/lib/use-push-notifications.ts
git commit -m "feat(push): add usePushNotifications hook"
```

---

### Task 12: Add "Notifications" toggle in Settings UI

**Objective:** Add a settings section where the user can enable/disable push notifications.

**Files:**
- Create: `web/src/components/settings/sections/notifications-section.tsx`
- Modify: `web/src/app/(main)/settings/page.tsx` (add tab + section)

**Step 1: Create the section component**

```tsx
// web/src/components/settings/sections/notifications-section.tsx
"use client";

import { usePushNotifications } from "@/lib/use-push-notifications";

export function NotificationsSection() {
  const { permission, subscribed, loading, enable, disable } = usePushNotifications();

  return (
    <div className="space-y-4">
      <h2 className="text-lg font-semibold">Browser Notifications</h2>
      <p className="text-sm text-text-secondary">
        Get native notifications on this device when frona has updates — even when the tab is closed.
      </p>

      {permission === "unsupported" && (
        <p className="text-sm text-error">
          Push notifications are not supported in this browser.
        </p>
      )}

      {permission === "denied" && (
        <p className="text-sm text-error">
          Notifications are blocked. Please enable them in your browser settings
          and reload this page.
        </p>
      )}

      {permission === "granted" && subscribed && (
        <div className="flex items-center gap-3">
          <span className="text-sm text-success">✓ Notifications enabled on this device</span>
          <button
            onClick={disable}
            disabled={loading}
            className="text-sm text-error hover:underline disabled:opacity-50"
          >
            Disable
          </button>
        </div>
      )}

      {permission !== "granted" && permission !== "unsupported" && permission !== "denied" && (
        <button
          onClick={enable}
          disabled={loading}
          className="px-4 py-2 bg-accent text-white rounded-lg text-sm hover:opacity-90 disabled:opacity-50"
        >
          {loading ? "Enabling..." : "Enable Notifications"}
        </button>
      )}
    </div>
  );
}
```

**Step 2: Add the tab to settings page**

In `web/src/app/(main)/settings/page.tsx`:

Add to the `TABS` array (after `profile`):
```typescript
  { id: "notifications", label: "Notifications", group: "user", saveable: false },
```

Add the import:
```typescript
import { NotificationsSection } from "@/components/settings/sections/notifications-section";
```

Add to the section rendering switch (follow the pattern of other sections like `<ProfileSection />`):
```tsx
  {activeTab === "notifications" && <NotificationsSection />}
```

**Step 3: Verify the frontend builds**

Run: `cd web && npm run build`
Expected: builds with no errors

**Step 4: Commit**

```bash
git add web/src/components/settings/sections/notifications-section.tsx web/src/app/(main)/settings/page.tsx
git commit -m "feat(push): add notifications settings section with enable/disable toggle"
```

---

## Phase 4: Integration & Testing

### Task 13: Generate VAPID keys and configure

**Objective:** Generate the VAPID key pair and add to frona config.

**Step 1: Generate VAPID keys**

Run (in the `web/` directory — npx is available via the existing Node.js toolchain):
```bash
cd web && npx web-push generate-vapid-keys
```

Expected output:
```
Public Key:  <base64url string>
Private Key: <base64url string>
```

**Step 2: Add to frona config**

Set as environment variables (or in `config.yaml` under `push:`):

```bash
# PowerShell
$env:FRONA_PUSH_VAPID_PUBLIC_KEY = "<public key from step 1>"
$env:FRONA_PUSH_VAPID_PRIVATE_KEY = "<private key from step 1>"
$env:FRONA_PUSH_SUBJECT = "https://myfrona.morganpercy.com"
```

Or in `config.yaml`:
```yaml
push:
  vapid_public_key: "<public key>"
  vapid_private_key: "<private key>"
  subject: "https://myfrona.morganpercy.com"
```

**Step 3: Restart frona server and verify it logs push is enabled**

Expected in server logs:
```
INFO ... Push notifications enabled (VAPID configured)
```

(If no VAPID keys, it should log: `Push notifications disabled (no VAPID keys configured)` — add this log to `AppState::new()` if not already.)

---

### Task 14: End-to-end test on desktop Chrome

**Objective:** Verify push notifications work end-to-end on desktop Chrome.

**Step 1: Start frona in dev mode**

```bash
cd ~/GitHub/frona
cargo run --bin frona
```

```bash
cd ~/GitHub/frona/web
npm run dev
```

**Step 2: Open browser and enable notifications**

1. Navigate to `http://localhost:3000`
2. Go to Settings → Notifications
3. Click "Enable Notifications"
4. Verify the browser permission prompt appears
5. Click "Allow"
6. Verify the toggle shows "✓ Notifications enabled on this device"

**Step 3: Trigger a notification**

1. Send a message to an agent (triggers `send_message.rs` notification)
2. Or start/stop an app (triggers `manage_app.rs` notification)
3. Verify:
   - In-app notification appears (existing SSE path — should still work)
   - A native OS notification appears (new push path)
4. Click the notification — should focus the frona tab and navigate to the deep link

**Step 4: Test background delivery**

1. Navigate away from the frona tab (or minimize the browser)
2. Trigger a notification from another device or via API
3. Verify the native notification appears even though the tab is not focused

**Step 5: Commit any fixes**

---

### Task 15: E2E test on Android Chrome + cleanup

**Objective:** Verify push works on mobile (Android Chrome) and handle expired subscriptions.

**Step 1: Android Chrome test**

1. On an Android phone, open Chrome and navigate to `https://myfrona.morganpercy.com`
2. Go to Settings → Notifications → Enable
3. Close the tab (or background Chrome)
4. Trigger a notification from desktop
5. Verify the notification appears in the Android notification shade
6. Tap it — should open Chrome to the deep link

**Step 2: iOS Safari PWA test (optional)**

1. On iOS, open Safari and navigate to `https://myfrona.morganpercy.com`
2. Share → Add to Home Screen
3. Open the PWA from home screen
4. Settings → Notifications → Enable
5. Close the PWA
6. Trigger a notification — verify it appears as an iOS notification

**Step 3: Verify expired subscription cleanup**

1. Manually unsubscribe a subscription via browser dev tools (Application → Service Workers → Push Subscription → Unsubscribe)
2. Trigger a notification
3. Check server logs — should see `Removing expired push subscription` for that endpoint
4. Verify the subscription is deleted from DB

---

## Risks, Tradeoffs, and Open Questions

### Risks
- **`web-push` crate API churn**: The crate has had breaking changes across versions. The implementer must verify the exact API for version 0.10 on docs.rs. Key construction (`PartialVapidSignature`) may differ.
- **iOS Safari restrictions**: Push only works on iOS 16.4+ when the site is installed as a PWA. Not a bug — a platform limitation.
- **Android permission gating**: `Notification.requestPermission()` must be called from a user gesture (click), not on page load. The `usePushNotifications` hook's `enable()` is called from a button click, which satisfies this.
- **VAPID key format**: Keys must be base64url-encoded uncompressed P-256 ECDSA keys. The `npx web-push generate-vapid-keys` output is already in the correct format.
- **Endpoint dedup**: The `subscribe` endpoint does delete-then-create instead of upsert. If the browser sends the same endpoint twice, the old subscription is replaced. This is fine but means the `created_at` timestamp updates on re-subscription.

### Tradeoffs
- **Fire-and-forget push**: Push send is `tokio::spawn`'d — it doesn't block notification creation, but it also means push failures are silently logged. This matches the SSE pattern (also fire-and-forget).
- **Graceful degradation**: If VAPID keys are not configured, `push_sender` is `None` and push is silently disabled. SSE (in-app) notifications still work. This is the right default for dev/testing.
- **One subscription per device, not per browser**: We dedup by endpoint URL. If a user has two tabs open in the same browser, they get one subscription. This is correct — the Service Worker handles delivery.

### Open Questions
- **Notification icon**: Should we add a custom frona icon (`/icon-192.png`)? The SW references it but it doesn't exist yet. The implementer can create a placeholder or remove the `icon`/`badge` lines from the SW.
- **Payload size**: Web Push payloads are limited to ~4KB. Our JSON payload is well under this. No need to truncate.
- **Rate limiting**: If the user has many subscriptions (10+ devices), sending push to all of them on every notification could be slow. The fire-and-forget pattern means this doesn't block, but it does create outbound HTTP requests. If this becomes a problem, batch or throttle later.
- **Config UI for VAPID keys**: The plan puts VAPID keys in env vars / config.yaml. Should the settings UI also allow admins to set them via the config page? This can be added later if needed.

---

## Files Summary

### New files (backend)
- `crates/frona-server/src/notification/push_model.rs` — PushSubscription entity
- `crates/frona-server/src/notification/push_repository.rs` — trait
- `crates/frona-server/src/notification/push_sender.rs` — VAPID signing + send
- `crates/frona-server/src/db/repo/push_subscriptions.rs` — SurrealDB impl
- `crates/frona-server/src/api/routes/push.rs` — REST endpoints
- `crates/frona-server/tests/api/push.rs` — tests

### New files (frontend)
- `web/public/sw.js` — service worker
- `web/src/lib/sw-register.ts` — SW registration helper
- `web/src/lib/use-push-notifications.ts` — React hook
- `web/src/components/settings/sections/notifications-section.tsx` — settings UI

### Modified files (backend)
- `crates/frona-server/src/notification/mod.rs` — new modules
- `crates/frona-server/src/notification/service.rs` — `create_and_notify()`
- `crates/frona-server/src/db/repo/mod.rs` — new module
- `crates/frona-server/src/core/config.rs` — `PushConfig` struct
- `crates/frona-server/src/core/state.rs` — `push_sender` + `push_subscription_repo`
- `crates/frona-server/src/api/routes/mod.rs` — new module
- `crates/frona-server/src/main.rs` — merge push router
- `crates/frona-server/Cargo.toml` — `web-push` crate
- `crates/frona-server/src/tool/send_message.rs` — use `create_and_notify()`
- `crates/frona-server/src/tool/manage_app.rs` — use `create_and_notify()`
- `crates/frona-server/src/core/supervisor.rs` — use `create_and_notify()`

### Modified files (frontend)
- `web/src/app/(main)/layout.tsx` — SW registration
- `web/src/app/(main)/settings/page.tsx` — notifications tab

### Config
- `config.yaml` or env vars — `FRONA_PUSH_VAPID_PUBLIC_KEY`, `FRONA_PUSH_VAPID_PRIVATE_KEY`, `FRONA_PUSH_SUBJECT`