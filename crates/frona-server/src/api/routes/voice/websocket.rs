use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequest, Query, Request, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::call::models::CallDirection;
use crate::chat::broadcast::BroadcastEventKind;
use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::inference::conversation::DefaultConversationBuilder;
use crate::inference::{InferenceEventKind, InferenceResponse};
use crate::tool::voice::{VoiceSessionExtensions, find_user_by_phone};

use super::models::TokenQuery;
use super::verify_voice_jwt;

/// Shorthand for the write half of the ConversationRelay socket. Shared
/// between the turn, the token streamer, and the silence filler.
type WsSend = Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>;

/// Spoken before hanging up when the agent called `hangup_call` without a
/// closing line of its own, so the call never ends on abrupt silence.
const DEFAULT_HANGUP_SIGN_OFF: &str = "Thanks for calling. Goodbye.";

/// Spoken before a transfer when the agent called `transfer_call` without a
/// closing line of its own — same purpose as `DEFAULT_HANGUP_SIGN_OFF`.
const DEFAULT_TRANSFER_SIGN_OFF: &str = "One moment, I'll connect you now.";

/// Prefix a session's first prompt with context the agent needs before it
/// can usefully reply: who's calling, and — if this leg was reached via
/// `transfer_call` — why it's picking up mid-call. `transfer_note` takes
/// priority since a transferred leg also carries `caller_name`.
fn prefix_first_prompt(
    voice_prompt: String,
    caller_name: Option<&str>,
    caller_phone: Option<&str>,
    transfer_note: Option<&str>,
) -> String {
    let phone = caller_phone.unwrap_or("unknown");
    if let Some(note) = transfer_note {
        let name = caller_name.unwrap_or("the caller");
        format!(
            "[CALL_TRANSFERRED: You're picking up a live call. Caller: {name} ({phone}). Handoff note: {note}.]\n{voice_prompt}"
        )
    } else if let Some(name) = caller_name {
        format!("[INBOUND_CALL: Incoming call from {name} ({phone}).]\n{voice_prompt}")
    } else {
        voice_prompt
    }
}

/// Parse `TransferCallTool`'s JSON result (`{"target_agent_id":..,
/// "note":..}`) into `(target_agent_id, note)`. `None` on malformed JSON —
/// callers fall back to an empty target, which `connect_action` treats as
/// "no transfer" (ends the call) rather than panicking on a value that
/// should never actually be malformed in practice.
fn parse_transfer_result(result: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(result).ok()?;
    let target_agent_id = v.get("target_agent_id")?.as_str()?.to_string();
    let note = v.get("note").and_then(|n| n.as_str()).unwrap_or_default().to_string();
    Some((target_agent_id, note))
}

/// Default filler phrases used when `silence_fill_phrases` is empty.
const DEFAULT_SILENCE_FILL_PHRASES: &[&str] = &[
    "Just a moment, I'm working on that.",
    "Let me look into that for you.",
    "Still thinking — bear with me.",
    "One moment please.",
    "I'm still processing your request.",
];

pub(crate) async fn twilio_ws_handler(
    State(state): State<AppState>,
    Query(q): Query<TokenQuery>,
    req: Request,
) -> Response {
    let claims = match verify_voice_jwt(&state, &q.token).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "Voice WS JWT verification failed");
            return (StatusCode::FORBIDDEN, "Invalid token").into_response();
        }
    };

    let ext: VoiceSessionExtensions = match claims
        .extensions
        .clone()
        .ok_or_else(|| AppError::Validation("voice session token missing extensions".into()))
        .and_then(|v| {
            serde_json::from_value(v)
                .map_err(|e| AppError::Validation(format!("voice session extensions: {e}")))
        }) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(error = %e, "Voice WS token extensions invalid");
            return (StatusCode::BAD_REQUEST, "Invalid voice token payload").into_response();
        }
    };

    let chat_id = ext.chat_id.clone();
    let user_id = claims.sub.clone();
    let contact_id = ext.contact_id.clone();
    let call_id = ext.call_id.clone();
    let caller_name = ext.caller_name.clone();
    let caller_phone = ext.caller_phone.clone();
    let transfer_note = ext.transfer_note.clone();
    // `direction` is only set for inbound calls (see VoiceSessionExtensions).
    let is_inbound = matches!(ext.direction, Some(CallDirection::Inbound));

    let ws = match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    ws.on_upgrade(move |socket| {
        handle_voice_socket(
            socket, state, chat_id, user_id, contact_id, call_id, caller_name, caller_phone,
            transfer_note, is_inbound,
        )
    })
}

#[allow(clippy::too_many_arguments)]
async fn handle_voice_socket(
    socket: WebSocket,
    state: AppState,
    chat_id: String,
    user_id: String,
    contact_id: Option<String>,
    call_id: Option<String>,
    caller_name: Option<String>,
    caller_phone: Option<String>,
    transfer_note: Option<String>,
    is_inbound: bool,
) {
    let (mut session_id, _) = state.active_sessions.register(&chat_id).await;
    tracing::debug!(chat_id = %chat_id, "Voice WS session registered in active sessions");

    // The timer-based silence filler speaks generic canned phrases, which are
    // appropriate only when the other party is someone we know — not an
    // arbitrary third party, where the agent narrates its own progress instead
    // (see active_call.md) and canned phrases would sound robotic.
    //
    // Inbound callers already cleared the per-user allowlist before the call
    // was answered, so they are known by definition — an allowlisted friend or
    // colleague need not also be a registered user. Only outbound calls need
    // the number lookup, which also keeps it off the inbound answer path.
    let remote_is_known = if is_inbound {
        true
    } else {
        match caller_phone.as_deref() {
            Some(phone) => find_user_by_phone(&state.user_service, phone).await.is_some(),
            None => false,
        }
    };
    tracing::debug!(chat_id = %chat_id, is_inbound, remote_is_known, "Silence-fill gating resolved");

    let (ws_send, mut ws_recv) = socket.split();
    // Wrap the send half in Arc<Mutex> so it can be shared between the agent
    // turn task and the silence-filler task.
    let ws_send = Arc::new(Mutex::new(ws_send));
    let mut last_response = String::new();
    let mut first_prompt = true;
    // Set when the turn loop already handled why this socket is closing
    // (`hangup_call` already marked the call completed; `transfer_call`
    // deliberately doesn't, since the call isn't over). Anything else that
    // ends this socket — the caller hanging up, the relay dropping — leaves
    // it to the cleanup at the bottom.
    let mut relay_closed_cleanly = false;

    loop {
        let msg = match ws_recv.next().await {
            Some(Ok(Message::Text(raw))) => raw,
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(_)) => continue,
            Some(Err(e)) => {
                tracing::warn!(error = %e, chat_id = %chat_id, "Voice WS receive error");
                break;
            }
        };

        let parsed: Value = match serde_json::from_str(&msg) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = parsed["type"].as_str().unwrap_or("").to_string();
        tracing::debug!(chat_id = %chat_id, msg_type = %msg_type, "Voice WS message received");

        match msg_type.as_str() {
            "setup" => {
                tracing::info!(chat_id = %chat_id, user_id = %user_id, contact_id = ?contact_id, "ConversationRelay connected");
            }
            "interrupt" => {
                tracing::debug!(chat_id = %chat_id, "ConversationRelay interrupt — cancelling active turn");
                state.active_sessions.cancel(&chat_id).await;
            }
            "prompt" if parsed["last"].as_bool() == Some(true) => {
                let voice_prompt = match parsed["voicePrompt"].as_str() {
                    Some(s) if !s.is_empty() => s.to_string(),
                    _ => {
                        tracing::debug!(chat_id = %chat_id, "Ignoring prompt with empty voicePrompt");
                        continue;
                    }
                };

                tracing::info!(chat_id = %chat_id, prompt = %voice_prompt, "Voice turn starting");
                let (turn_id, cancel_token) = state.active_sessions.register(&chat_id).await;
                session_id = turn_id;

                // On the first prompt of the session, prepend context the
                // agent needs: who's calling, and — if this leg was reached
                // via transfer_call — why it's picking up mid-call.
                let effective_prompt = if first_prompt {
                    first_prompt = false;
                    prefix_first_prompt(
                        voice_prompt,
                        caller_name.as_deref(),
                        caller_phone.as_deref(),
                        transfer_note.as_deref(),
                    )
                } else {
                    voice_prompt
                };

                // --- Token streaming ---
                // Subscribe before the turn starts so the first delta can't be
                // published before we're listening. Everything the agent says
                // reaches the caller through this task; the turn itself only
                // closes it out below.
                let bus_rx = state.broadcast_service.subscribe_raw();
                let stream_stop = CancellationToken::new();
                let last_activity = Arc::new(StdMutex::new(Instant::now()));
                let stream_handle = spawn_turn_streamer(
                    bus_rx,
                    ws_send.clone(),
                    stream_stop.clone(),
                    last_activity.clone(),
                    chat_id.clone(),
                );

                // --- Silence filler ---
                // Spawn a background task that periodically sends filler phrases
                // to the caller while the agent is processing. The filler is
                // cancelled when the turn completes (or errors).
                let filler_cancel = CancellationToken::new();
                let filler_handle = if remote_is_known && state.config.voice.silence_fill_enabled {
                    let ws = ws_send.clone();
                    let fc = filler_cancel.clone();
                    let cid = chat_id.clone();
                    let initial = Duration::from_secs(
                        state.config.voice.silence_fill_initial_delay_secs.max(1),
                    );
                    let interval = Duration::from_secs(
                        state.config.voice.silence_fill_interval_secs.max(1),
                    );
                    let phrases = if state.config.voice.silence_fill_phrases.is_empty() {
                        DEFAULT_SILENCE_FILL_PHRASES
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        state.config.voice.silence_fill_phrases.clone()
                    };
                    Some(tokio::spawn(silence_filler(
                        ws,
                        fc,
                        initial,
                        interval,
                        phrases,
                        last_activity.clone(),
                        cid,
                    )))
                } else {
                    None
                };

                let (response_text, outcome) = match handle_voice_turn(
                    &state,
                    &user_id,
                    &chat_id,
                    &effective_prompt,
                    cancel_token,
                    ws_send.clone(),
                    contact_id.as_deref(),
                    call_id.as_deref(),
                )
                .await
                {
                    Ok(result) => result,
                    Err(e) => {
                        tracing::error!(error = %e, chat_id = %chat_id, "Voice turn failed");
                        filler_cancel.cancel();
                        if let Some(h) = filler_handle {
                            let _ = h.await;
                        }
                        // Close out anything already streamed so the relay
                        // isn't left waiting on a turn that will never end.
                        stream_stop.cancel();
                        let streamed = stream_handle.await.unwrap_or(StreamedTurn {
                            text: String::new(),
                            first_token_at: None,
                            replay: None,
                        });
                        if !streamed.text.is_empty() {
                            end_turn(&ws_send, &chat_id).await;
                        }
                        continue;
                    }
                };

                // Stop the filler and the streamer, and wait for both to finish
                // so they release the ws_send lock before we close the turn.
                filler_cancel.cancel();
                if let Some(h) = filler_handle {
                    let _ = h.await;
                }
                stream_stop.cancel();
                let streamed = stream_handle.await.unwrap_or(StreamedTurn {
                    text: String::new(),
                    first_token_at: None,
                    replay: None,
                });

                tracing::info!(chat_id = %chat_id, response_len = %response_text.len(), streamed_len = %streamed.text.len(), outcome = ?outcome, "Voice turn complete");

                // Remember the agent's own words for the task summary — never the
                // canned sign-off below, which would otherwise overwrite the last
                // meaningful thing the agent said.
                if !response_text.is_empty() {
                    last_response = response_text.clone();
                }

                // Whatever streamed has already reached the caller, so the only
                // thing left is to close the turn. Text still lands here when
                // nothing streamed — a provider that returned without emitting
                // deltas, or a hangup whose sign-off the model omitted, which
                // would otherwise leave the caller hearing the line go dead.
                let unspoken = if !streamed.text.is_empty() {
                    ""
                } else if !response_text.trim().is_empty() {
                    response_text.as_str()
                } else {
                    match &outcome {
                        TurnOutcome::Hangup => {
                            tracing::info!(chat_id = %chat_id, "Hangup with no closing line — using default sign-off");
                            DEFAULT_HANGUP_SIGN_OFF
                        }
                        TurnOutcome::Transfer { .. } => {
                            tracing::info!(chat_id = %chat_id, "Transfer with no closing line — using default sign-off");
                            DEFAULT_TRANSFER_SIGN_OFF
                        }
                        TurnOutcome::Continue => "",
                    }
                };

                if !unspoken.is_empty() || !streamed.text.is_empty() {
                    if !unspoken.is_empty() {
                        tracing::debug!(chat_id = %chat_id, response = %unspoken, "Sending unstreamed TTS response");
                    }
                    let tts = serde_json::json!({
                        "type": "text",
                        "token": unspoken,
                        "last": true
                    });
                    {
                        let mut send = ws_send.lock().await;
                        if send
                            .send(Message::Text(tts.to_string().into()))
                            .await
                            .is_err()
                        {
                            tracing::warn!(chat_id = %chat_id, "Failed to send TTS response — closing");
                            break;
                        }
                    }
                }

                if !matches!(outcome, TurnOutcome::Continue) {
                    relay_closed_cleanly = true;
                    // Wait on what was actually spoken, so the sign-off isn't
                    // cut off by hanging up too early. Streamed speech has been
                    // playing since the first token, so discount the time the
                    // caller has already spent listening to it.
                    let spoken_text = if streamed.text.is_empty() {
                        unspoken
                    } else {
                        streamed.text.as_str()
                    };
                    let word_count = spoken_text.split_whitespace().count();
                    let tts_secs = ((word_count as f64 / 2.5).ceil() as u64 + 1).clamp(2, 30);
                    let already_played = streamed
                        .first_token_at
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0);
                    let tts_secs = tts_secs.saturating_sub(already_played).max(1);
                    tracing::info!(chat_id = %chat_id, tts_secs, already_played, "Waiting for TTS before ending the relay session");

                    // A transfer embeds its target/note in handoffData so
                    // connect_action (api::routes::voice::mod) can pick up
                    // the hand-off with no DB round-trip; a plain hangup
                    // sends the same "end" it always has.
                    let end_msg = match &outcome {
                        TurnOutcome::Transfer { target_agent_id, note } => {
                            tracing::info!(chat_id = %chat_id, target_agent_id = %target_agent_id, "Sending transfer signal to Twilio");
                            let handoff_data = serde_json::json!({
                                "target_agent_id": target_agent_id,
                                "note": note,
                            })
                            .to_string();
                            serde_json::json!({ "type": "end", "handoffData": handoff_data })
                        }
                        _ => {
                            tracing::info!(chat_id = %chat_id, "Sending hangup signal to Twilio");
                            serde_json::json!({ "type": "end" })
                        }
                    };
                    {
                        let mut send = ws_send.lock().await;
                        send.send(Message::Text(end_msg.to_string().into())).await.ok();
                    }
                    break;
                }
            }
            "prompt" => {
                tracing::debug!(chat_id = %chat_id, "Ignoring partial prompt (last=false)");
            }
            other => {
                tracing::debug!(chat_id = %other, msg_type = %other, "Unhandled ConversationRelay message type");
            }
        }
    }

    tracing::info!(chat_id = %chat_id, "Voice WS session ended");
    state.active_sessions.remove(&chat_id, session_id).await;

    // The socket closing is the only signal we get when the *other* party
    // hangs up: Twilio just drops the relay. Without this the call row would
    // sit at `Active` for ever, since only the agent's own `hangup_call`
    // completes it. A transfer also closes this socket without ending the
    // call, so `relay_closed_cleanly` (set for either outcome) covers both.
    if let Some(cid) = call_id.as_deref()
        && !relay_closed_cleanly
        && let Err(e) = state.call_service.mark_completed(cid).await
    {
        tracing::warn!(error = %e, call_id = %cid, "Failed to mark call completed on socket close");
    }

    if let Ok(Some(task)) = state.task_service.find_by_chat_id(&chat_id).await
        && matches!(task.status, crate::agent::task::models::TaskStatus::InProgress)
    {
        // The last thing spoken is a sign-off ("Thanks, goodbye"), not a useful
        // report of what the call achieved. Summarise the transcript instead,
        // falling back to the last utterance if that isn't possible.
        let summary = summarise_call(&state, &chat_id, &task.agent_id, &task.user_id)
            .await
            .unwrap_or(last_response);

        if let Ok(task) = state.task_service.mark_completed(&task.id, Some(summary.clone())).await {
            crate::agent::task::executor::deliver_event_to_source(
                &state.chat_service,
                &task,
                crate::agent::task::executor::TaskLifecycleEvent::Completion {
                    status: crate::agent::task::models::TaskStatus::Completed,
                    summary: Some(summary),
                    citations: Vec::new(),
                },
                vec![],
            )
            .await;
            state.task_executor.resume_parent_if_requested(&task).await;
        }
    }
}

/// Build the transcript of a call chat as alternating speaker lines. Returns
/// `None` when there is nothing worth summarising.
fn render_call_transcript(messages: &[crate::chat::message::models::Message]) -> Option<String> {
    use crate::chat::message::models::MessageRole;

    let mut lines = Vec::new();
    for m in messages {
        let speaker = match m.role {
            // What the other party said, and the agent's spoken replies.
            MessageRole::LiveCall | MessageRole::User | MessageRole::Contact => "Other party",
            MessageRole::Agent => "Assistant",
            // Task/system bookkeeping isn't part of the conversation.
            MessageRole::System | MessageRole::TaskCompletion => continue,
        };
        let text = m.content.trim();
        if !text.is_empty() {
            lines.push(format!("{speaker}: {text}"));
        }
    }

    // A single line is just the greeting or the sign-off — nothing to condense.
    if lines.len() < 2 {
        return None;
    }
    Some(lines.join("\n"))
}

/// Summarise a finished call for the user, who wasn't on it. Returns `None` on
/// any failure so the caller can fall back to the last thing spoken — a call
/// that happened must never be lost because summarising it failed.
async fn summarise_call(
    state: &AppState,
    chat_id: &str,
    agent_id: &str,
    user_id: &str,
) -> Option<String> {
    use crate::inference::usage::models::{CompactionTarget, InferenceKind, UsageContext};

    let messages = state.chat_service.get_stored_messages(chat_id).await.ok()?;
    let transcript = render_call_transcript(&messages)?;

    let prompt = state.prompts.read("CALL_SUMMARY.md")?;
    let model_group = state
        .chat_service
        .provider_registry()
        .utility_model_group("call_summary")
        .ok()?;

    let usage_ctx = UsageContext::new(
        InferenceKind::Compaction {
            target: CompactionTarget::Chat {
                agent_id: agent_id.to_string(),
                chat_id: chat_id.to_string(),
            },
        },
        user_id,
        model_group.name.clone(),
    );

    match crate::inference::text_inference(
        state.chat_service.provider_registry(),
        &model_group,
        &prompt,
        vec![rig_core::completion::Message::user(transcript)],
        &state.usage_service,
        &usage_ctx,
    )
    .await
    {
        Ok(summary) if !summary.trim().is_empty() => {
            tracing::info!(chat_id = %chat_id, "Call summarised for task completion");
            Some(summary.trim().to_string())
        }
        Ok(_) => None,
        Err(e) => {
            tracing::warn!(error = %e, chat_id = %chat_id, "Call summarisation failed; using last utterance");
            None
        }
    }
}

/// What the caller actually heard during a turn, as streamed by
/// [`spawn_turn_streamer`].
struct StreamedTurn {
    /// Concatenation of every text delta forwarded to the relay. Equal to the
    /// turn's final text, since the deltas are what the text is built from.
    text: String,
    /// When the first delta went out — i.e. when the caller started hearing
    /// speech. Used to avoid over-waiting for TTS before a hangup.
    first_token_at: Option<Instant>,
    /// Set while a retried attempt is re-streaming text the caller has already
    /// heard. `None` during normal streaming.
    replay: Option<Replay>,
}

/// Tracks a retried attempt so its re-streamed prefix isn't spoken twice.
///
/// A retry restarts the turn's text from the beginning, so everything up to
/// what the caller already heard is dropped and only the excess is forwarded.
struct Replay {
    /// How much of the turn the caller heard before the retry began. Counted in
    /// chars so slicing the replayed text can't split a UTF-8 boundary.
    spoken_chars: usize,
    /// Text accumulated since the retry began.
    attempt: String,
}

/// A bus event the streamer acts on.
enum TurnEvent {
    /// A text delta to speak.
    Text(String),
    /// Inference is retrying — whatever follows re-streams the turn from the
    /// start (see `inference::retry::stream_with_retry_and_fallback`).
    Retry,
}

/// Forward the agent's text deltas to ConversationRelay as they are produced,
/// so TTS starts on the first words instead of after the whole agent loop
/// (tool rounds included) has finished.
///
/// Inference publishes each delta on the broadcast bus as
/// `InferenceEventKind::Text` (see `inference::retry`); concatenating them
/// reproduces the turn text exactly, so the caller hears the reply once and
/// only once. Each delta goes out as `last: false` — the caller closes the
/// turn with a single `last: true` when the agent loop is done.
///
/// The bus is subscribed by the caller *before* the turn starts so no early
/// delta is missed, and drained on stop so no late one is dropped.
fn spawn_turn_streamer(
    mut bus_rx: tokio::sync::mpsc::UnboundedReceiver<crate::chat::broadcast::BroadcastEvent>,
    ws_send: WsSend,
    stop: CancellationToken,
    last_activity: Arc<StdMutex<Instant>>,
    chat_id: String,
) -> tokio::task::JoinHandle<StreamedTurn> {
    tokio::spawn(async move {
        let mut turn = StreamedTurn {
            text: String::new(),
            first_token_at: None,
            replay: None,
        };

        // Classify a bus event, if it is one of ours. Reasoning deltas are
        // deliberately ignored — the caller must never hear the agent's
        // private thinking.
        let classify = |event: crate::chat::broadcast::BroadcastEvent| -> Option<TurnEvent> {
            if event.chat_id.as_deref() != Some(chat_id.as_str()) {
                return None;
            }
            match event.kind {
                BroadcastEventKind::Inference(InferenceEventKind::Text(t)) if !t.is_empty() => {
                    Some(TurnEvent::Text(t))
                }
                BroadcastEventKind::Inference(InferenceEventKind::Retry { .. }) => {
                    Some(TurnEvent::Retry)
                }
                _ => None,
            }
        };

        loop {
            let event = tokio::select! {
                _ = stop.cancelled() => break,
                ev = bus_rx.recv() => match ev {
                    Some(ev) => ev,
                    None => break,
                },
            };
            if let Some(event) = classify(event)
                && !handle_event(&ws_send, &mut turn, &last_activity, &chat_id, event).await
            {
                tracing::warn!(chat_id = %chat_id, "Token stream: send failed — stopping");
                return turn;
            }
        }

        // The turn is over, but deltas published just before the stop may still
        // be queued. Publishing is synchronous into this unbounded channel, so
        // everything the turn produced is already here — draining it is what
        // keeps the tail of the reply from being cut off.
        while let Ok(event) = bus_rx.try_recv() {
            if let Some(event) = classify(event)
                && !handle_event(&ws_send, &mut turn, &last_activity, &chat_id, event).await
            {
                break;
            }
        }

        tracing::debug!(chat_id = %chat_id, streamed_len = turn.text.len(), "Token stream complete");
        turn
    })
}

/// Apply one classified bus event. Returns whether the socket is still
/// writable.
async fn handle_event(
    ws_send: &WsSend,
    turn: &mut StreamedTurn,
    last_activity: &StdMutex<Instant>,
    chat_id: &str,
    event: TurnEvent,
) -> bool {
    let delta = match event {
        TurnEvent::Retry => {
            // The next attempt starts the turn's text over. Anything the caller
            // already heard must not be spoken a second time.
            let spoken_chars = turn.text.chars().count();
            tracing::info!(chat_id = %chat_id, spoken_chars, "Inference retry — suppressing replayed speech");
            turn.replay = Some(Replay {
                spoken_chars,
                attempt: String::new(),
            });
            return true;
        }
        TurnEvent::Text(delta) => delta,
    };

    match next_utterance(turn, delta) {
        Some(text) => send_delta(ws_send, turn, last_activity, text).await,
        // Wholly inside what the caller already heard — say nothing.
        None => true,
    }
}

/// Decide what a text delta should actually put on the wire, given whether a
/// retried attempt is currently replaying text the caller already heard.
///
/// Returns `None` when the delta is entirely a replay and must stay silent.
fn next_utterance(turn: &mut StreamedTurn, delta: String) -> Option<String> {
    // Not replaying — the delta goes out as-is.
    let Some(replay) = turn.replay.as_mut() else {
        return Some(delta);
    };

    replay.attempt.push_str(&delta);
    if replay.attempt.chars().count() <= replay.spoken_chars {
        return None;
    }

    // The retry has caught up to where the caller was left. Speak the excess
    // and resume normal streaming. A retry that diverges from the first attempt
    // resumes mid-thought rather than repeating the opening, which is the
    // lesser of the two artefacts.
    let excess: String = replay.attempt.chars().skip(replay.spoken_chars).collect();
    turn.replay = None;
    Some(excess)
}

/// Close the open TTS turn with an empty final token, telling the relay no more
/// tokens are coming for it.
async fn end_turn(ws_send: &WsSend, chat_id: &str) {
    let tts = serde_json::json!({
        "type": "text",
        "token": "",
        "last": true
    });
    let mut send = ws_send.lock().await;
    if send.send(Message::Text(tts.to_string().into())).await.is_err() {
        tracing::warn!(chat_id = %chat_id, "Failed to close TTS turn");
    }
}

/// Send one text delta to the relay as a non-final token. Returns whether the
/// socket is still writable.
async fn send_delta(
    ws_send: &WsSend,
    turn: &mut StreamedTurn,
    last_activity: &StdMutex<Instant>,
    delta: String,
) -> bool {
    if turn.first_token_at.is_none() {
        turn.first_token_at = Some(Instant::now());
    }
    turn.text.push_str(&delta);
    if let Ok(mut at) = last_activity.lock() {
        *at = Instant::now();
    }

    let msg = serde_json::json!({
        "type": "text",
        "token": delta,
        "last": false
    });
    let mut send = ws_send.lock().await;
    send.send(Message::Text(msg.to_string().into()))
        .await
        .is_ok()
}

/// Periodically sends filler phrases to the caller while the agent is
/// processing a turn. Stops when `cancel` is triggered.
///
/// Each filler phrase is sent as `{"type":"text","token":"…","last":true}`
/// so ConversationRelay speaks it immediately, closing whatever turn is open.
///
/// Since the agent's own words now stream out token by token, canned filler is
/// only wanted for genuine dead air — a long tool call, say. `last_activity`
/// tracks when a real token last went out, and a cycle that lands inside that
/// quiet window is skipped rather than spoken, so filler never talks over the
/// agent mid-sentence.
async fn silence_filler(
    ws_send: WsSend,
    cancel: CancellationToken,
    initial_delay: Duration,
    interval: Duration,
    phrases: Vec<String>,
    last_activity: Arc<StdMutex<Instant>>,
    chat_id: String,
) {
    // Wait the initial silence period before sending the first filler.
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(initial_delay) => {}
    }

    let mut idx: usize = 0;
    loop {
        // Real speech went out recently — the caller isn't sitting in silence,
        // so say nothing this cycle and re-check after the interval.
        let quiet_for = last_activity
            .lock()
            .map(|at| at.elapsed())
            .unwrap_or(initial_delay);
        if quiet_for < initial_delay {
            tokio::select! {
                _ = cancel.cancelled() => return,
                _ = tokio::time::sleep(interval) => continue,
            }
        }

        // Pick a phrase — simple rotating index avoids extra dependencies.
        let phrase = if phrases.is_empty() {
            return;
        } else {
            let p = &phrases[idx % phrases.len()];
            idx += 1;
            p.clone()
        };

        let filler_msg = serde_json::json!({
            "type": "text",
            "token": phrase,
            "last": true
        });

        {
            let mut send = ws_send.lock().await;
            if send
                .send(Message::Text(filler_msg.to_string().into()))
                .await
                .is_err()
            {
                tracing::warn!(chat_id = %chat_id, "Silence filler: failed to send — stopping");
                return;
            }
        }
        tracing::info!(chat_id = %chat_id, phrase = %phrase, "Silence filler sent");

        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

/// What a completed voice turn means for the socket — whether to keep
/// talking, or how to close the relay session (a plain hangup, or a
/// transfer that hands the chat to a different agent).
#[derive(Debug)]
enum TurnOutcome {
    Continue,
    Hangup,
    /// Carried in the ConversationRelay `end` message's `handoffData` so
    /// `connect_action` (api::routes::voice::mod) can pick up the hand-off
    /// without a DB round-trip.
    Transfer { target_agent_id: String, note: String },
}

#[allow(clippy::too_many_arguments)]
async fn handle_voice_turn(
    state: &AppState,
    user_id: &str,
    chat_id: &str,
    content: &str,
    cancel_token: CancellationToken,
    ws_send: Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    contact_id: Option<&str>,
    call_id: Option<&str>,
) -> Result<(String, TurnOutcome), AppError> {
    state
        .chat_service
        .save_live_call_message(user_id, chat_id, content, contact_id)
        .await?;

    let chat = state.chat_service.find_chat(chat_id).await?
        .ok_or_else(|| AppError::NotFound("Chat not found".into()))?;

    loop {
        // Create or find an Executing agent message for this turn
        let agent_msg_id = match state.chat_service
            .find_executing_message_for_chat(chat_id)
            .await
        {
            Ok(Some(msg)) => msg.id,
            _ => {
                let msg = state.chat_service
                    .create_executing_agent_message(chat_id, &chat.agent_id)
                    .await?;
                msg.id
            }
        };

        let builder = Box::new(DefaultConversationBuilder {
            user_service: state.user_service.clone(),
            storage_service: state.storage_service.clone(),
            agent_service: state.agent_service.clone(),
        });
        let outcome = state.harness.run_loop(user_id, chat_id, &agent_msg_id, cancel_token.clone(), builder, &[], None).await?;
        let mut response = outcome.response;

        match outcome.inference {
            InferenceResponse::ExternalToolPending {
                ref tool_calls, ref turn_text, ..
            } if tool_calls.iter().any(|te| te.name == "send_dtmf") => {
                let tool_call = tool_calls.iter().find(|te| te.name == "send_dtmf").unwrap();
                tracing::debug!(chat_id = %chat_id, digits = %tool_call.result, "Sending DTMF digits");

                let dtmf_msg = serde_json::json!({
                    "type": "sendDigits",
                    "digits": tool_call.result
                });
                {
                    let mut send = ws_send.lock().await;
                    send.send(Message::Text(dtmf_msg.to_string().into())).await.ok();
                }

                let _ = state.chat_service
                    .resolve_tool_call(&tool_call.id, Some("DTMF sent".to_string()))
                    .await;

                response.content = turn_text.clone();
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;
            }
            InferenceResponse::ExternalToolPending {
                ref tool_calls, ref turn_text, ..
            } if tool_calls.iter().any(|te| te.name == "hangup_call") => {
                let tool_call = tool_calls.iter().find(|te| te.name == "hangup_call").unwrap();
                tracing::debug!(chat_id = %chat_id, "Hangup requested by agent");

                let _ = state.chat_service
                    .resolve_tool_call(&tool_call.id, Some("Call ended".to_string()))
                    .await;

                response.content = turn_text.clone();
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;

                if let Some(cid) = call_id
                    && let Err(e) = state.call_service.mark_completed(cid).await
                {
                    tracing::warn!(error = %e, call_id = %cid, "Failed to mark call completed");
                }

                return Ok((turn_text.clone(), TurnOutcome::Hangup));
            }
            InferenceResponse::ExternalToolPending {
                ref tool_calls, ref turn_text, ..
            } if tool_calls.iter().any(|te| te.name == "transfer_call") => {
                let tool_call = tool_calls.iter().find(|te| te.name == "transfer_call").unwrap();
                tracing::debug!(chat_id = %chat_id, "Transfer requested by agent");

                // TransferCallTool's result IS this JSON — see tool::voice.
                let (target_agent_id, note) = parse_transfer_result(&tool_call.result).unwrap_or_else(|| {
                    tracing::error!(chat_id = %chat_id, result = %tool_call.result, "Failed to parse transfer_call result");
                    (String::new(), String::new())
                });

                let _ = state.chat_service
                    .resolve_tool_call(&tool_call.id, Some("Transfer initiated".to_string()))
                    .await;

                response.content = turn_text.clone();
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;

                // Deliberately not marking the call completed — it isn't
                // over, just handed to a different agent by connect_action.
                return Ok((turn_text.clone(), TurnOutcome::Transfer { target_agent_id, note }));
            }
            InferenceResponse::Completed { text, attachments, reasoning, .. } => {
                response.content = text.clone();
                response.attachments = attachments;
                response.reasoning = reasoning;
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;
                return Ok((text, TurnOutcome::Continue));
            }
            InferenceResponse::ExternalToolPending {
                ref tool_calls, ref turn_text, ..
            } => {
                // The agent called a non-voice tool (search, browser, etc.) and
                // produced `turn_text` alongside the tool call. The streamer
                // has already sent that narration to the caller token by token,
                // so re-sending it here would say it twice.
                //
                // NOTE: We intentionally do NOT cancel the timer-based filler
                // here. Cancelling it on the first tool call killed it for
                // the rest of the turn and all subsequent thinking time.
                // The Arc<Mutex<ws_send>> already prevents overlapping sends,
                // and the filler's own interval provides natural spacing
                // between phrases.
                if !turn_text.is_empty() {
                    tracing::info!(chat_id = %chat_id, text = %turn_text, "Agent narration streamed to caller");
                }

                // Resolve tool calls and continue the loop for the next
                // inference round.
                for tc in tool_calls {
                    let _ = state.chat_service
                        .resolve_tool_call(&tc.id, Some("executed".to_string()))
                        .await;
                }
                response.content = turn_text.clone();
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;
                // Continue the loop — the harness will process the tool
                // results and produce the next inference.
            }
            _ => {
                let _ = state.chat_service
                    .fail_agent_message(response, "voice inference unexpected branch".to_string()).await;
                return Ok((String::new(), TurnOutcome::Continue));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transfer_prefix_takes_priority_over_inbound_prefix() {
        // A transferred leg carries both caller_name (from the original
        // inbound extensions) and transfer_note — the transfer prefix must
        // win, not the plain inbound one.
        let prefixed = prefix_first_prompt(
            "Hello?".to_string(),
            Some("Alice"),
            Some("+15555551234"),
            Some("Wants a refund on order #123"),
        );
        assert_eq!(
            prefixed,
            "[CALL_TRANSFERRED: You're picking up a live call. Caller: Alice (+15555551234). Handoff note: Wants a refund on order #123.]\nHello?"
        );
    }

    #[test]
    fn inbound_prefix_used_when_no_transfer_note() {
        let prefixed = prefix_first_prompt("Hello?".to_string(), Some("Alice"), Some("+15555551234"), None);
        assert_eq!(
            prefixed,
            "[INBOUND_CALL: Incoming call from Alice (+15555551234).]\nHello?"
        );
    }

    #[test]
    fn no_prefix_for_outbound_calls() {
        // Outbound calls carry neither caller_name nor transfer_note.
        let prefixed = prefix_first_prompt("Hello?".to_string(), None, None, None);
        assert_eq!(prefixed, "Hello?");
    }

    #[test]
    fn parse_transfer_result_extracts_target_and_note() {
        let result = r#"{"target_agent_id":"agent-123","note":"caller wants billing"}"#;
        assert_eq!(
            parse_transfer_result(result),
            Some(("agent-123".to_string(), "caller wants billing".to_string()))
        );
    }

    #[test]
    fn parse_transfer_result_defaults_missing_note_to_empty() {
        let result = r#"{"target_agent_id":"agent-123"}"#;
        assert_eq!(
            parse_transfer_result(result),
            Some(("agent-123".to_string(), String::new()))
        );
    }

    #[test]
    fn parse_transfer_result_none_on_malformed_json() {
        assert_eq!(parse_transfer_result("not json"), None);
        assert_eq!(parse_transfer_result(r#"{"note":"only a note"}"#), None);
    }

    /// A turn that has already spoken `spoken` to the caller.
    fn turn_after(spoken: &str) -> StreamedTurn {
        StreamedTurn {
            text: spoken.to_string(),
            first_token_at: None,
            replay: None,
        }
    }

    /// Mark the turn as retrying, as `handle_event` does on a Retry event.
    fn begin_replay(turn: &mut StreamedTurn) {
        turn.replay = Some(Replay {
            spoken_chars: turn.text.chars().count(),
            attempt: String::new(),
        });
    }

    #[test]
    fn deltas_pass_through_when_not_replaying() {
        let mut turn = turn_after("Hello");
        assert_eq!(
            next_utterance(&mut turn, " there".into()),
            Some(" there".to_string())
        );
    }

    #[test]
    fn replayed_prefix_is_not_spoken_twice() {
        let mut turn = turn_after("Your balance is");
        begin_replay(&mut turn);

        // The retry re-streams the same opening — none of it reaches the caller.
        assert_eq!(next_utterance(&mut turn, "Your ".into()), None);
        assert_eq!(next_utterance(&mut turn, "balance ".into()), None);
        assert_eq!(next_utterance(&mut turn, "is".into()), None);

        // Past what was spoken, only the excess goes out.
        assert_eq!(
            next_utterance(&mut turn, " forty pounds.".into()),
            Some(" forty pounds.".to_string())
        );
        // Replay is over — later deltas stream normally again.
        assert!(turn.replay.is_none());
        assert_eq!(
            next_utterance(&mut turn, " Anything else?".into()),
            Some(" Anything else?".to_string())
        );
    }

    #[test]
    fn delta_straddling_the_replay_boundary_speaks_only_the_excess() {
        let mut turn = turn_after("Your balance");
        begin_replay(&mut turn);

        // One delta carries both the replayed opening and new text.
        assert_eq!(
            next_utterance(&mut turn, "Your balance is forty.".into()),
            Some(" is forty.".to_string())
        );
    }

    #[test]
    fn replay_boundary_does_not_split_a_multibyte_char() {
        // "£" is multi-byte: slicing by bytes here would panic or corrupt.
        let mut turn = turn_after("Balance: £");
        begin_replay(&mut turn);

        assert_eq!(
            next_utterance(&mut turn, "Balance: £40".into()),
            Some("40".to_string())
        );
    }

    #[test]
    fn diverging_retry_resumes_instead_of_repeating() {
        let mut turn = turn_after("Let me check that");
        begin_replay(&mut turn);

        // The retry says something different. The caller hears the tail rather
        // than the opening a second time — resuming mid-phrase, but never
        // repeating what they already heard.
        let spoken = next_utterance(&mut turn, "One moment while I look it up.".into());
        assert_eq!(spoken, Some("I look it up.".to_string()));
    }
}
