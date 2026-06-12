//! `presage::Manager::receive_messages()` internally calls
//! `tokio::task::spawn_local`, so the manager MUST run on a `current_thread`
//! runtime. We give each channel its own OS thread + runtime and bridge to
//! the main multi-thread runtime through tokio mpsc channels.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::oneshot as futures_oneshot;
use presage::libsignal_service::configuration::SignalServers;
use presage::model::identity::OnNewIdentity;
use presage::model::messages::Received;
use presage::store::StateStore;
use presage::Manager;
use presage_store_sqlite::SqliteStore;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::chat::channel::ChannelManager;
use crate::chat::channel::models::{ChannelCtx, ExternalMessage};
use crate::core::error::AppError;

use super::command::{self, SignalCommand};
use super::convert;

pub const CMD_BUFFER: usize = 64;
pub const QR_TIMEOUT: Duration = Duration::from_secs(45);

/// `link_secondary_device` first calls `store.clear_registration()`, so
/// `on_setup_begin` short-circuits via this helper to avoid wiping a healthy
/// device when the manager retries after a WS interruption.
pub async fn is_already_registered(db_path: &Path) -> bool {
    if !db_path.exists() {
        return false;
    }
    let url = format!("sqlite://{}", db_path.display());
    let store = match SqliteStore::open(&url, OnNewIdentity::Trust).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                db_path = %db_path.display(),
                error = %e,
                "Signal is_already_registered: store open failed, treating as not-registered",
            );
            return false;
        }
    };
    matches!(store.load_registration_data().await, Ok(Some(_)))
}

/// Rust's `std::thread` default is 2 MiB, which overflows on the first poll
/// of `Manager::receive_messages` (presage state struct + flagged-large
/// futures). 8 MiB matches the Linux pthread default.
pub const WORKER_STACK_SIZE: usize = 8 * 1024 * 1024;

pub struct SignalHandle {
    pub cmd_tx: mpsc::Sender<SignalCommand>,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

/// `expect_setup = true` runs `link_secondary_device` and forwards the
/// `tsdevice:/…` URL through the returned oneshot; `false` calls
/// `load_registered` against the existing store.
pub async fn spawn(
    ctx: &ChannelCtx,
    device_name: String,
    expect_setup: bool,
) -> Result<(SignalHandle, Option<String>), AppError> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<SignalCommand>(CMD_BUFFER);

    let (qr_tx, qr_rx) = if expect_setup {
        let (tx, rx) = oneshot::channel::<String>();
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    let db_path = ctx.data_dir.join("store.db");
    let emit = ctx.emit.clone();
    let cancel = ctx.cancel.clone();
    let channel_id = ctx.channel.id.clone();
    let channel_manager = ctx.channel_manager.clone();
    let chat_service = ctx.chat_service.clone();
    let cmd_tx_inner = cmd_tx.clone();

    let thread = std::thread::Builder::new()
        .name(format!("signal-{channel_id}"))
        .stack_size(WORKER_STACK_SIZE)
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        channel_id = %channel_id,
                        error = %e,
                        "Signal worker failed to build current_thread runtime",
                    );
                    return;
                }
            };
            rt.block_on(run(
                db_path,
                device_name,
                expect_setup,
                qr_tx,
                cmd_rx,
                cmd_tx_inner,
                emit,
                cancel,
                channel_id,
                channel_manager,
                chat_service,
            ));
        })
        .map_err(|e| AppError::Internal(format!("Signal worker thread spawn: {e}")))?;

    let qr = match qr_rx {
        Some(rx) => match tokio::time::timeout(QR_TIMEOUT, rx).await {
            Ok(Ok(url)) => Some(url),
            Ok(Err(_)) => return Err(AppError::Internal(
                "Signal worker dropped QR oneshot before emitting a URL".into(),
            )),
            Err(_) => {
                return Err(AppError::Internal(format!(
                    "Signal link URL not emitted within {:?}",
                    QR_TIMEOUT
                )));
            }
        },
        None => None,
    };

    Ok((
        SignalHandle {
            cmd_tx,
            thread: Some(thread),
        },
        qr,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run(
    db_path: PathBuf,
    device_name: String,
    expect_setup: bool,
    qr_tx: Option<oneshot::Sender<String>>,
    mut cmd_rx: mpsc::Receiver<SignalCommand>,
    cmd_tx: mpsc::Sender<SignalCommand>,
    emit: mpsc::Sender<ExternalMessage>,
    cancel: CancellationToken,
    channel_id: String,
    cm: Arc<ChannelManager>,
    chat_service: crate::chat::service::ChatService,
) {
    let db_str = match db_path.to_str() {
        Some(s) => s.to_string(),
        None => {
            cm.report_failure(&channel_id, "Signal data_dir path is not UTF-8".into())
                .await;
            return;
        }
    };
    if let Some(parent) = db_path.parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        cm.report_failure(&channel_id, format!("Signal create data_dir: {e}"))
            .await;
        return;
    }

    let url = format!("sqlite://{db_str}");
    tracing::debug!(
        channel_id = %channel_id,
        db_path = %db_path.display(),
        expect_setup,
        "Signal opening SqliteStore",
    );
    let store = match SqliteStore::open(&url, OnNewIdentity::Trust).await {
        Ok(s) => s,
        Err(e) => {
            cm.report_failure(&channel_id, format!("Signal SqliteStore open ({url}): {e}"))
                .await;
            return;
        }
    };

    let mut manager = if expect_setup {
        let (link_tx, link_rx) = futures_oneshot::channel::<Url>();
        let qr_forward = async move {
            if let Ok(url) = link_rx.await
                && let Some(tx) = qr_tx
            {
                let _ = tx.send(url.to_string());
            }
        };
        let link_fut = Manager::link_secondary_device(
            store,
            SignalServers::Production,
            device_name,
            link_tx,
        );
        match futures::future::join(link_fut, qr_forward).await {
            (Ok(m), _) => {
                tracing::info!(
                    channel_id = %channel_id,
                    db_path = %db_path.display(),
                    "Signal device linked - presage keys persisted",
                );
                cm.report_setup_complete(&channel_id).await;
                m
            }
            (Err(e), _) => {
                cm.report_failure(
                    &channel_id,
                    format!("Signal link_secondary_device failed: {e}"),
                )
                .await;
                return;
            }
        }
    } else {
        match Manager::load_registered(store).await {
            Ok(m) => {
                tracing::info!(
                    channel_id = %channel_id,
                    db_path = %db_path.display(),
                    "Signal manager loaded from persisted store",
                );
                m
            }
            Err(e) => {
                cm.report_failure(
                    &channel_id,
                    format!("Signal load_registered failed: {e} (was the data dir wiped?)"),
                )
                .await;
                return;
            }
        }
    };

    let stream = match Box::pin(manager.receive_messages()).await {
        Ok(s) => s,
        Err(e) => {
            cm.report_failure(&channel_id, format!("Signal receive_messages init: {e}"))
                .await;
            return;
        }
    };
    tracing::info!(
        channel_id = %channel_id,
        "Signal receive stream opened - draining backlog",
    );
    futures::pin_mut!(stream);

    let mut ready = false;
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!(channel_id = %channel_id, "Signal worker cancelled");
                break;
            }
            rcv = stream.next() => {
                match rcv {
                    None => {
                        cm.report_failure(&channel_id, "Signal receive stream ended (websocket interrupted)".into()).await;
                        break;
                    }
                    Some(Received::QueueEmpty) => {
                        ready = true;
                        tracing::info!(channel_id = %channel_id, "Signal backlog drained - ready to send");
                    }
                    Some(Received::Contacts) => {
                        tracing::info!(channel_id = %channel_id, "Signal contacts sync received");
                    }
                    Some(Received::Content(content)) => {
                        convert::handle(
                            &mut manager,
                            &emit,
                            &cmd_tx,
                            *content,
                            &channel_id,
                            &chat_service,
                            &cm,
                        )
                        .await;
                    }
                }
            }
            Some(cmd) = cmd_rx.recv(), if ready => {
                command::handle(&mut manager, cmd, &channel_id).await;
            }
            else => break,
        }
    }
}
