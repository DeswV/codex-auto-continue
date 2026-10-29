//! Thread-local recovery owned by the server listener, including without subscribers.
use crate::outgoing_message::{OutgoingMessageSender, ThreadScopedOutgoingMessageSender};
use crate::thread_state::{ThreadState, ThreadStateManager};
use codex_app_server_protocol::{AutoRetryNotification, ServerNotification};
use codex_core::context::{
    ContextualUserFragment, InternalContextSource, InternalModelContextFragment,
};
use codex_core::{CodexThread, TurnInput, TurnInputRequest};
use codex_protocol::ThreadId;
use codex_protocol::protocol::{EventMsg, ThreadGoalStatus};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Default)]
pub(crate) struct AutoRetry {
    pending: Option<Pending>,
    cancelled_turn: Option<String>,
    last: Option<(Instant, Duration)>,
    ceiling: f64,
    reset: Option<i64>,
    provider_id: Option<String>,
    goal_active: bool,
    goal_turn_id: Option<String>,
    turn_goal: bool,
}
struct Pending {
    turn_id: String,
    deadline: Instant,
    started: Instant,
    retry_at: i64,
    capability: bool,
    goal: bool,
    ready: bool,
}
impl AutoRetry {
    pub(crate) fn notification(&self, thread_id: ThreadId) -> Option<AutoRetryNotification> {
        self.pending.as_ref().map(|p| AutoRetryNotification {
            thread_id: thread_id.to_string(),
            turn_id: p.turn_id.clone(),
            retry_at: Some(p.retry_at),
            reason: if p.capability {
                "Model capability error"
            } else {
                "5h usage limit reached"
            }
            .into(),
        })
    }
    pub(crate) fn observe_goal(&mut self, active: bool, turn_id: Option<&str>) {
        if (active || self.goal_active)
            && let Some(turn_id) = turn_id
        {
            self.goal_turn_id = Some(turn_id.to_owned());
        }
        self.goal_active = active;
        if active {
            self.turn_goal = true;
        }
    }
    pub(crate) fn seed_goal(&mut self, active: bool) {
        self.goal_active = active;
        self.turn_goal = active;
    }
    pub(crate) fn is_pending(&self) -> bool {
        self.pending.is_some()
    }
    pub(crate) fn tick_delay(&self) -> Option<Duration> {
        self.pending.as_ref().map(|p| {
            if p.ready {
                p.deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(1))
            } else {
                Duration::from_secs(1)
            }
        })
    }
    pub(crate) fn cancel(&mut self, turn_id: &str) -> bool {
        if self
            .pending
            .as_ref()
            .is_some_and(|p| p.turn_id == turn_id || turn_id.is_empty())
        {
            self.cancelled_turn = self.pending.as_ref().map(|p| p.turn_id.clone());
            self.pending = None;
            self.last = None;
            true
        } else {
            false
        }
    }
    pub(crate) fn observe(&mut self, turn_id: &str, event: &EventMsg) {
        match event {
            EventMsg::ThreadSettingsApplied(e) => {
                let provider = &e.thread_settings.model_provider_id;
                if self.provider_id.as_ref().is_some_and(|id| id != provider) {
                    self.reset = None;
                    self.last = None;
                    self.pending = None;
                }
                self.provider_id = Some(provider.clone());
            }
            EventMsg::ThreadGoalUpdated(e) => {
                self.observe_goal(
                    e.goal.status == ThreadGoalStatus::Active,
                    e.turn_id.as_deref(),
                );
            }
            EventMsg::TurnStarted(_) => {
                self.cancelled_turn = None;
                self.pending = None;
                self.turn_goal = self.goal_active || self.goal_turn_id.as_deref() == Some(turn_id);
            }
            EventMsg::TokenCount(e) => {
                if let Some(s) = &e.rate_limits {
                    if s.limit_id
                        .as_deref()
                        .is_none_or(|id| id.eq_ignore_ascii_case("codex"))
                    {
                        self.reset = s
                            .primary
                            .iter()
                            .chain(s.secondary.iter())
                            .find(|w| w.window_minutes == Some(300) && w.used_percent >= 100.0)
                            .and_then(|w| w.resets_at);
                    }
                }
            }
            EventMsg::Error(e) if e.affects_turn_status() => {
                if self.cancelled_turn.as_deref() == Some(turn_id) {
                    return;
                }
                let message = e.message.to_ascii_lowercase();
                let capability =
                    message.contains("model capability") || message.contains("model_capability");
                let now = Instant::now();
                let duration = if capability {
                    let Some(ceiling) = next_ceiling(self.last, self.ceiling, now) else {
                        return;
                    };
                    self.ceiling = ceiling;
                    Duration::from_secs_f64(rand::random::<f64>() * ceiling)
                } else if message.contains("usage limit") || message.contains("5h") {
                    let Some(reset) = self.reset.or_else(|| parse_reset(&message)) else {
                        return;
                    };
                    Duration::from_secs((reset - chrono::Utc::now().timestamp()).max(1) as u64)
                } else {
                    return;
                };
                self.pending = Some(Pending {
                    turn_id: turn_id.into(),
                    deadline: now + duration,
                    started: now,
                    retry_at: chrono::Utc::now().timestamp_millis() + duration.as_millis() as i64,
                    capability,
                    goal: self.turn_goal,
                    ready: false,
                });
            }
            EventMsg::TurnComplete(_) => {
                if let Some(p) = self.pending.as_mut() {
                    if p.turn_id == turn_id {
                        p.ready = true;
                    }
                }
            }
            EventMsg::TurnAborted(_) | EventMsg::ShutdownComplete => {
                self.pending = None;
            }
            _ => {}
        }
    }
}
pub(crate) async fn cancel(
    id: ThreadId,
    turn_id: &str,
    manager: &ThreadStateManager,
    outgoing: &Arc<OutgoingMessageSender>,
) -> bool {
    let Some(state) = manager.existing_thread_state(id).await else {
        return false;
    };
    let notification = {
        let mut state = state.lock().await;
        let notification = state.auto_retry.notification(id);
        if !state.auto_retry.cancel(turn_id) {
            return false;
        }
        notification
    };
    if let Some(mut notification) = notification {
        notification.retry_at = None;
        let sender = ThreadScopedOutgoingMessageSender::new(
            Arc::clone(outgoing),
            manager.subscribed_connection_ids(id).await,
            id,
        );
        sender
            .send_server_notification(ServerNotification::AutoRetry(notification))
            .await;
    }
    true
}

pub(crate) async fn wait(delay: Option<Duration>) {
    match delay {
        Some(d) => tokio::time::sleep(d).await,
        None => std::future::pending().await,
    }
}
pub(crate) async fn tick(
    id: ThreadId,
    thread: &Arc<CodexThread>,
    state: &Arc<tokio::sync::Mutex<ThreadState>>,
    goals: &codex_goal_extension::GoalService,
    manager: &ThreadStateManager,
    outgoing: &Arc<OutgoingMessageSender>,
) {
    // Keep cancellation serialized through admission. continue_turn_if_idle also rejects
    // a newer user turn, even if it completed before this timer woke up.
    let connections = manager.subscribed_connection_ids(id).await;
    let mut state = state.lock().await;
    let Some(p) = state.auto_retry.pending.as_ref() else {
        return;
    };
    let due = p.ready && Instant::now() >= p.deadline;
    let notification = AutoRetryNotification {
        thread_id: id.to_string(),
        turn_id: p.turn_id.clone(),
        retry_at: if due { None } else { Some(p.retry_at) },
        reason: if p.capability {
            "Model capability error"
        } else {
            "5h usage limit reached"
        }
        .into(),
    };
    let sender = ThreadScopedOutgoingMessageSender::new(Arc::clone(outgoing), connections, id);
    sender
        .send_server_notification(ServerNotification::AutoRetry(notification))
        .await;
    if !due {
        return;
    }
    let p = state.auto_retry.pending.take().unwrap();
    if p.capability {
        state.auto_retry.last = Some((Instant::now(), p.started.elapsed()));
    }
    if p.goal {
        if let Some(db) = thread.state_db() {
            use codex_goal_extension::{
                GoalObjectiveUpdate, GoalSetRequest, GoalTokenBudgetUpdate,
            };
            // Do not revive completed or budget-limited goals.
            if let Ok(Some(goal)) = goals.get_thread_goal(&db, id).await {
                if !matches!(
                    goal.status,
                    ThreadGoalStatus::Paused
                        | ThreadGoalStatus::Blocked
                        | ThreadGoalStatus::UsageLimited
                        | ThreadGoalStatus::Active
                ) {
                    return;
                }
                match goals
                    .set_thread_goal(
                        &db,
                        GoalSetRequest {
                            thread_id: id,
                            objective: GoalObjectiveUpdate::Keep,
                            status: Some(ThreadGoalStatus::Active),
                            token_budget: GoalTokenBudgetUpdate::Keep,
                            max_goal_token_budget: None,
                        },
                        async { Ok(()) },
                    )
                    .await
                {
                    Ok(outcome) => {
                        state.auto_retry.seed_goal(true);
                        if let Err(error) = thread
                            .append_rollout_items(&[outcome.thread_goal_updated_item()])
                            .await
                        {
                            tracing::warn!(%error, "failed to persist auto-retry goal resume");
                        }
                        sender
                            .send_server_notification(ServerNotification::ThreadGoalUpdated(
                                codex_app_server_protocol::ThreadGoalUpdatedNotification {
                                    thread_id: id.to_string(),
                                    turn_id: None,
                                    goal: outcome.goal.clone().into(),
                                },
                            ))
                            .await;
                        outcome.apply_runtime_effects(goals).await;
                    }
                    Err(error) => tracing::warn!(%error, "auto-retry goal resume failed"),
                }
            }
        }
    } else {
        let input = ContextualUserFragment::into(InternalModelContextFragment::new(
            InternalContextSource::from_static("auto_retry"),
            "Continue",
        ));
        match thread
            .continue_turn_if_idle(
                TurnInputRequest::new(TurnInput::ResponseItem(input)),
                p.turn_id.clone(),
            )
            .await
        {
            Ok(codex_core::TurnInputSubmission::NotSubmitted {
                reason: codex_protocol::turn_input::NotSubmittedReason::NotIdle,
            }) => {
                // TurnComplete can be delivered just before Core clears its active slot.
                // Retain this recovery until admission is possible or a newer turn cancels it.
                let mut pending = p;
                pending.deadline = Instant::now() + Duration::from_millis(100);
                state.auto_retry.pending = Some(pending);
                if let Some(notification) = state.auto_retry.notification(id) {
                    sender
                        .send_server_notification(ServerNotification::AutoRetry(notification))
                        .await;
                }
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "auto-retry continuation failed"),
        }
    }
}
fn next_ceiling(last: Option<(Instant, Duration)>, ceiling: f64, now: Instant) -> Option<f64> {
    match last.filter(|(end, _)| now.duration_since(*end) <= Duration::from_secs(64)) {
        Some((_, wait)) if wait > Duration::from_secs(64) => None,
        Some(_) => Some(ceiling * 2.0),
        None => Some(0.5),
    }
}

fn parse_reset(message: &str) -> Option<i64> {
    use chrono::{Local, NaiveDateTime, NaiveTime, TimeZone};
    let text = message.split("try again at ").nth(1)?.trim_end_matches('.');
    let text = text
        .replace("st,", ",")
        .replace("nd,", ",")
        .replace("rd,", ",")
        .replace("th,", ",");
    let text = text.to_uppercase();
    let local = NaiveDateTime::parse_from_str(&text, "%b %e, %Y %I:%M %p")
        .ok()
        .or_else(|| {
            NaiveTime::parse_from_str(&text, "%I:%M %p")
                .ok()
                .map(|t| Local::now().date_naive().and_time(t))
        })?;
    Local
        .from_local_datetime(&local)
        .single()
        .map(|dt| dt.timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::protocol::{ErrorEvent, TurnCompleteEvent};
    fn error(message: &str) -> EventMsg {
        EventMsg::Error(ErrorEvent {
            message: message.into(),
            codex_error_info: None,
            misalignment: None,
        })
    }
    fn complete() -> EventMsg {
        EventMsg::TurnComplete(
            serde_json::from_value::<TurnCompleteEvent>(
                serde_json::json!({"turn_id":"failed", "last_agent_message":null}),
            )
            .unwrap(),
        )
    }
    #[test]
    fn exponential_backoff_boundaries() {
        let now = Instant::now();
        assert_eq!(next_ceiling(None, 99.0, now), Some(0.5));
        assert_eq!(
            next_ceiling(Some((now, Duration::from_secs(64))), 128.0, now),
            Some(256.0)
        );
        assert_eq!(
            next_ceiling(
                Some((now, Duration::from_secs(65))),
                128.0,
                now + Duration::from_secs(64)
            ),
            None
        );
        assert_eq!(
            next_ceiling(
                Some((now, Duration::from_secs(65))),
                128.0,
                now + Duration::from_secs(65)
            ),
            Some(0.5)
        );
    }
    #[tokio::test(start_paused = true)]
    async fn deadline_survives_time_without_clients_and_reconnect() {
        let mut state = AutoRetry {
            reset: Some(chrono::Utc::now().timestamp() + 300),
            ..Default::default()
        };
        state.observe("failed", &error("usage limit reached"));
        state.observe("failed", &complete());
        let id = ThreadId::new();
        let initial = state.notification(id).unwrap();
        tokio::time::advance(Duration::from_secs(120)).await;
        // A new subscriber receives the same deadline, not a freshly sampled wait.
        let reconnected = state.notification(id).unwrap();
        assert_eq!(initial, reconnected);
        assert!(state.pending.as_ref().unwrap().deadline > Instant::now());
        tokio::time::advance(Duration::from_secs(181)).await;
        assert_eq!(state.tick_delay(), Some(Duration::ZERO));
        assert!(state.pending.as_ref().unwrap().ready);
    }
    #[tokio::test(start_paused = true)]
    async fn waits_for_terminal_event_and_can_cancel_without_clients() {
        let mut state = AutoRetry::default();
        state.observe("failed", &error("model capability error"));
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(!state.pending.as_ref().unwrap().ready);
        state.observe("failed", &complete());
        assert_eq!(state.tick_delay(), Some(Duration::ZERO));
        assert!(!state.cancel("different-turn"));
        assert!(state.cancel("failed"));
        assert_eq!(state.tick_delay(), None);
        state.observe("failed", &error("model capability error"));
        assert!(!state.is_pending());
    }
    #[test]
    fn goal_error_keeps_goal_resume_even_when_goal_event_arrives_first() {
        let mut state = AutoRetry::default();
        state.seed_goal(true);
        state.observe_goal(false, Some("failed"));
        let started = EventMsg::TurnStarted(
            serde_json::from_value(
                serde_json::json!({"turn_id":"failed", "model_context_window":null}),
            )
            .unwrap(),
        );
        state.observe("failed", &started);
        state.observe("failed", &error("model capability error"));
        assert!(state.pending.as_ref().unwrap().goal);
        let started = EventMsg::TurnStarted(
            serde_json::from_value(
                serde_json::json!({"turn_id":"new-user-turn", "model_context_window":null}),
            )
            .unwrap(),
        );
        state.observe("new-user-turn", &started);
        assert!(!state.is_pending());
        state.observe("new-user-turn", &error("model capability error"));
        assert!(!state.pending.as_ref().unwrap().goal);
    }

    #[test]
    fn parses_reset_message() {
        assert!(parse_reset("try again at oct 29th, 2026 11:30 pm.").is_some());
        assert!(parse_reset("try again at 11:30 pm.").is_some());
        assert_eq!(parse_reset("try again later."), None);
    }
}
