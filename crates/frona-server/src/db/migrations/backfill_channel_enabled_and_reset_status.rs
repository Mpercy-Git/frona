//! Backfills `enabled` and resets stale connection state on existing channel rows.
//! Runtime status isn't meaningful across a deploy, so we keep operator intent
//! (`enabled`) and the terminal `Failed` state, reset transient status to
//! `Disconnected`, and clear stale QR overlays; the pairing overlay is left untouched
//! and supervisors rebuild live status on boot. Re-entrant (see the per-step notes);
//! statuses are bound as typed `ChannelStatus` values, not string literals, to match
//! SurrealValue's on-disk encoding.

use surrealdb::Surreal;
use surrealdb::engine::local::Db;

use frona_derive::migration;

use crate::chat::channel::ChannelStatus;

#[migration("2026-07-08T00:00:00Z")]
async fn backfill_channel_enabled_and_reset_status(
    db: &Surreal<Db>,
) -> Result<(), surrealdb::Error> {
    // 1. Backfill intent from the pre-migration status: anything that was running
    //    (or terminally Failed) stays enabled; only an explicitly Disconnected
    //    channel is treated as intentionally off. `enabled IS NONE` makes this run
    //    exactly once, so step 2 resetting status never feeds back into intent.
    db.query("UPDATE channel SET enabled = (status != $disconnected) WHERE enabled IS NONE")
        .bind(("disconnected", ChannelStatus::Disconnected))
        .await?
        .check()?;

    // 2. Reset every transient/retired status (Connecting/Connected, the removed
    //    Setup/Pairing, and any stray Reconnecting) to Disconnected. Failed is
    //    terminal and preserved; Disconnected is already the target.
    db.query(
        "UPDATE channel SET status = $disconnected \
         WHERE status != $disconnected AND status != $failed",
    )
    .bind(("disconnected", ChannelStatus::Disconnected))
    .bind(("failed", ChannelStatus::Failed))
    .await?
    .check()?;

    // 3. Drop stale device-link (QR) overlays; a fresh QR is re-minted on the next
    //    connect if the channel is still unlinked.
    db.query("UPDATE channel SET setup = NONE")
        .await?
        .check()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use surrealdb::engine::local::Mem;

    async fn mem_db() -> Surreal<Db> {
        let db = Surreal::new::<Mem>(()).await.unwrap();
        db.use_ns("test").use_db("test").await.unwrap();
        crate::db::init::setup_schema(&db).await.unwrap();
        db
    }

    /// Seed a row with a status but no `enabled` field — the shape this migration backfills.
    async fn seed(db: &Surreal<Db>, id: &str, status: ChannelStatus, with_setup: bool) {
        let setup = if with_setup { "{ instructions: 'scan me' }" } else { "NONE" };
        db.query(format!(
            "CREATE type::record('channel', $id) SET status = $status, setup = {setup}"
        ))
        .bind(("id", id.to_string()))
        .bind(("status", status))
        .await
        .unwrap()
        .check()
        .unwrap();
    }

    async fn enabled_of(db: &Surreal<Db>, id: &str) -> Option<bool> {
        db.query("SELECT VALUE enabled FROM type::record('channel', $id)")
            .bind(("id", id.to_string()))
            .await
            .unwrap()
            .take(0)
            .unwrap()
    }

    async fn status_of(db: &Surreal<Db>, id: &str) -> ChannelStatus {
        db.query("SELECT VALUE status FROM type::record('channel', $id)")
            .bind(("id", id.to_string()))
            .await
            .unwrap()
            .take::<Option<ChannelStatus>>(0)
            .unwrap()
            .unwrap()
    }

    async fn setup_is_none(db: &Surreal<Db>, id: &str) -> bool {
        let raw: Option<serde_json::Value> = db
            .query("SELECT VALUE setup FROM type::record('channel', $id)")
            .bind(("id", id.to_string()))
            .await
            .unwrap()
            .take(0)
            .unwrap();
        raw.is_none()
    }

    #[tokio::test]
    async fn backfills_intent_resets_transient_and_clears_setup() {
        let db = mem_db().await;
        seed(&db, "connected", ChannelStatus::Connected, true).await;
        seed(&db, "connecting", ChannelStatus::Connecting, false).await;
        seed(&db, "disconnected", ChannelStatus::Disconnected, false).await;
        seed(&db, "failed", ChannelStatus::Failed, true).await;

        backfill_channel_enabled_and_reset_status(&db).await.unwrap();

        // Intent: running-ish + Failed → enabled; explicitly Disconnected → off.
        assert_eq!(enabled_of(&db, "connected").await, Some(true));
        assert_eq!(enabled_of(&db, "connecting").await, Some(true));
        assert_eq!(enabled_of(&db, "disconnected").await, Some(false));
        assert_eq!(enabled_of(&db, "failed").await, Some(true));

        // Transient runtime status is reset; Failed is preserved as terminal.
        assert_eq!(status_of(&db, "connected").await, ChannelStatus::Disconnected);
        assert_eq!(status_of(&db, "connecting").await, ChannelStatus::Disconnected);
        assert_eq!(status_of(&db, "disconnected").await, ChannelStatus::Disconnected);
        assert_eq!(status_of(&db, "failed").await, ChannelStatus::Failed);

        // Stale QR overlays cleared.
        assert!(setup_is_none(&db, "connected").await);
        assert!(setup_is_none(&db, "failed").await);
    }

    #[tokio::test]
    async fn is_reentrant_intent_survives_a_second_run() {
        let db = mem_db().await;
        seed(&db, "connected", ChannelStatus::Connected, true).await;

        backfill_channel_enabled_and_reset_status(&db).await.unwrap();
        assert_eq!(enabled_of(&db, "connected").await, Some(true));
        assert_eq!(status_of(&db, "connected").await, ChannelStatus::Disconnected);

        // Re-running must NOT recompute intent from the now-Disconnected status
        // (the `enabled IS NONE` guard); a naive backfill would flip it to false.
        backfill_channel_enabled_and_reset_status(&db).await.unwrap();
        assert_eq!(enabled_of(&db, "connected").await, Some(true));
        assert_eq!(status_of(&db, "connected").await, ChannelStatus::Disconnected);
    }
}
