use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{FromRequest, Query, Request, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::core::error::AppError;
use crate::core::state::AppState;
use crate::inference::InferenceResponse;
use crate::inference::conversation::DefaultConversationBuilder;
use crate::tool::voice::VoiceSessionExtensions;

use super::models::TokenQuery;
use super::verify_voice_jwt;

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

    let ws = match WebSocketUpgrade::from_request(req, &state).await {
        Ok(ws) => ws,
        Err(e) => return e.into_response(),
    };

    ws.on_upgrade(move |socket| handle_voice_socket(socket, state, chat_id, user_id, contact_id, call_id, caller_name, caller_phone))
}

async fn handle_voice_socket(
    socket: WebSocket,
    state: AppState,
    chat_id: String,
    user_id: String,
    contact_id: Option<String>,
    call_id: Option<String>,
    caller_name: Option<String>,
    caller_phone: Option<String>,
) {
    state.active_sessions.register(&chat_id).await;
    tracing::debug!(chat_id = %chat_id, "Voice WS session registered in active sessions");
    let (ws_send, mut ws_recv) = socket.split();
    // Wrap the send half in Arc<Mutex> so it can be shared between the agent
    // turn task and the silence-filler task.
    let ws_send = Arc::new(Mutex::new(ws_send));
    let mut last_response = String::new();
    let mut first_prompt = true;

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
                let cancel_token = state.active_sessions.register(&chat_id).await;

                // On the first prompt of an inbound call, prepend the caller
                // identity so the agent knows who's calling.
                let effective_prompt = if first_prompt {
                    first_prompt = false;
                    if let Some(ref name) = caller_name {
                        let phone = caller_phone.as_deref().unwrap_or("unknown");
                        format!("[INBOUND_CALL: Incoming call from {name} ({phone}).]\n{voice_prompt}")
                    } else {
                        voice_prompt
                    }
                } else {
                    voice_prompt
                };

                // --- Silence filler ---
                // Spawn a background task that periodically sends filler phrases
                // to the caller while the agent is processing. The filler is
                // cancelled when the turn completes (or errors).
                let filler_cancel = CancellationToken::new();
                let filler_handle = if state.config.voice.silence_fill_enabled {
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
                        ws, fc, initial, interval, phrases, cid,
                    )))
                } else {
                    None
                };

                let (response_text, should_hang_up) = match handle_voice_turn(
                    &state,
                    &user_id,
                    &chat_id,
                    &effective_prompt,
                    cancel_token,
                    ws_send.clone(),
                    contact_id.as_deref(),
                    call_id.as_deref(),
                    &filler_cancel,
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
                        continue;
                    }
                };

                // Stop the filler and wait for it to finish so it releases the
                // ws_send lock before we send the final TTS response.
                filler_cancel.cancel();
                if let Some(h) = filler_handle {
                    let _ = h.await;
                }

                tracing::info!(chat_id = %chat_id, response_len = %response_text.len(), should_hang_up = %should_hang_up, "Voice turn complete");
                if !response_text.is_empty() {
                    last_response = response_text.clone();
                    tracing::debug!(chat_id = %chat_id, response = %response_text, "Sending TTS response");
                    let tts = serde_json::json!({
                        "type": "text",
                        "token": response_text,
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

                if should_hang_up {
                    let word_count = response_text.split_whitespace().count();
                    let tts_secs = ((word_count as f64 / 2.5).ceil() as u64 + 1).clamp(2, 30);
                    tracing::info!(chat_id = %chat_id, tts_secs, "Waiting for TTS before hangup");
                    tokio::time::sleep(Duration::from_secs(tts_secs)).await;
                    tracing::info!(chat_id = %chat_id, "Sending hangup signal to Twilio");
                    let end_msg = serde_json::json!({ "type": "end" });
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
    state.active_sessions.remove(&chat_id).await;

    if let Ok(Some(task)) = state.task_service.find_by_chat_id(&chat_id).await
        && matches!(task.status, crate::agent::task::models::TaskStatus::InProgress)
    {
        let summary = last_response;

        if let Ok(task) = state.task_service.mark_completed(&task.id, Some(summary.clone())).await {
            crate::agent::task::executor::deliver_event_to_source(
                &state.chat_service,
                &task,
                crate::agent::task::executor::TaskLifecycleEvent::Completion {
                    status: crate::agent::task::models::TaskStatus::Completed,
                    summary: Some(summary),
                },
                vec![],
            )
            .await;
            state.task_executor.resume_parent_if_requested(&task).await;
        }
    }
}

/// Periodically sends filler phrases to the caller while the agent is
/// processing a turn. Stops when `cancel` is triggered.
///
/// Each filler phrase is sent as `{"type":"text","token":"…","last":true}`
/// so ConversationRelay speaks it immediately. The agent's real response
/// is sent later as another `last: true` message — ConversationRelay
/// handles multiple `last: true` messages in a single turn by queuing
/// them sequentially.
async fn silence_filler(
    ws_send: Arc<Mutex<futures::stream::SplitSink<WebSocket, Message>>>,
    cancel: CancellationToken,
    initial_delay: Duration,
    interval: Duration,
    phrases: Vec<String>,
    chat_id: String,
) {
    // Wait the initial silence period before sending the first filler.
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(initial_delay) => {}
    }

    let mut idx: usize = 0;
    loop {
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
    filler_cancel: &CancellationToken,
) -> Result<(String, bool), AppError> {
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

                return Ok((turn_text.clone(), true));
            }
            InferenceResponse::Completed { text, attachments, reasoning, .. } => {
                response.content = text.clone();
                response.attachments = attachments;
                response.reasoning = reasoning;
                let _ = state.chat_service
                    .complete_agent_message(response)
                    .await;
                return Ok((text, false));
            }
            InferenceResponse::ExternalToolPending {
                ref tool_calls, ref turn_text, ..
            } => {
                // The agent called a non-voice tool (search, browser, etc.)
                // and produced `turn_text` alongside the tool call. Send it
                // to the caller as TTS so they know what the agent is doing.
                if !turn_text.is_empty() {
                    // Pause the timer-based filler so it doesn't overlap.
                    filler_cancel.cancel();

                    let tts = serde_json::json!({
                        "type": "text",
                        "token": turn_text,
                        "last": true
                    });
                    {
                        let mut send = ws_send.lock().await;
                        send.send(Message::Text(tts.to_string().into())).await.ok();
                    }
                    tracing::info!(chat_id = %chat_id, text = %turn_text, "Agent narration sent to caller");
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
                return Ok((String::new(), false));
            }
        }
    }
}
