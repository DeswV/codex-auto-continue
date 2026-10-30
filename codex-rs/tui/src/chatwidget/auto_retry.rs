//! Presentation only: the app-server owns retry policy, deadlines and continuation.
use super::*;

#[derive(Default)]
pub(super) struct AutoRetry {
    cancelled_turn: Option<String>,
    pub(super) pending: Option<codex_app_server_protocol::AutoRetryNotification>,
}
impl ChatWidget {
    pub(crate) fn handle_auto_retry_exit_key(&mut self, key: KeyEvent) -> bool {
        if self.auto_retry.pending.is_some()
            && self.bottom_pane.composer_is_empty()
            && key.kind != KeyEventKind::Release
            && key.code == KeyCode::Char('c')
            && key.modifiers == KeyModifiers::CONTROL
        {
            self.app_event_tx
                .send(AppEvent::Exit(crate::app_event::ExitMode::Immediate));
            return true;
        }
        false
    }

    pub(super) fn clear_auto_retry(&mut self) {
        self.auto_retry.pending = None;
        self.bottom_pane
            .set_placeholder_text(self.normal_placeholder_text.clone());
        self.request_redraw();
    }
    pub(crate) fn cancel_auto_retry(&mut self) {
        if let Some(pending) = self.auto_retry.pending.take() {
            self.auto_retry.cancelled_turn = Some(pending.turn_id.clone());
            if let Some(thread_id) = self.thread_id {
                self.app_event_tx.send(AppEvent::CancelAutoRetry {
                    thread_id,
                    turn_id: pending.turn_id,
                });
            }
            self.clear_auto_retry();
        }
    }
    pub(super) fn on_auto_retry(
        &mut self,
        notification: codex_app_server_protocol::AutoRetryNotification,
    ) {
        if self.auto_retry.cancelled_turn.as_deref() == Some(&notification.turn_id) {
            return;
        }
        if notification.retry_at.is_none() {
            self.clear_auto_retry();
            return;
        }
        self.auto_retry.pending = Some(notification);
        if !self.bottom_pane.composer_is_empty() {
            self.cancel_auto_retry();
            return;
        }
        self.tick_auto_retry();
    }
    pub(super) fn tick_auto_retry(&mut self) {
        let Some(p) = &self.auto_retry.pending else {
            return;
        };
        let seconds =
            ((p.retry_at.unwrap_or_default() - chrono::Utc::now().timestamp_millis()).max(0) + 999)
                / 1000;
        self.bottom_pane.set_placeholder_text(format!("{}. Automatically continuing in {} h {} m {} s. Type to cancel; Ctrl+C to leave running in background.", p.reason, seconds / 3600, seconds / 60 % 60, seconds % 60));
        self.frame_requester
            .schedule_frame_in(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chatwidget::tests::make_chatwidget_manual_with_sender;
    fn notification() -> codex_app_server_protocol::AutoRetryNotification {
        codex_app_server_protocol::AutoRetryNotification {
            thread_id: "thread".into(),
            turn_id: "failed-turn".into(),
            retry_at: Some(chrono::Utc::now().timestamp_millis() + 120_000),
            reason: "5h usage limit reached".into(),
        }
    }
    #[tokio::test]
    async fn newly_connected_tui_uses_server_deadline() {
        let (mut widget, _, _, _) = make_chatwidget_manual_with_sender().await;
        let status = notification();
        widget.on_auto_retry(status.clone());
        assert_eq!(
            widget.auto_retry.pending.as_ref().unwrap().retry_at,
            status.retry_at
        );
        widget.tick_auto_retry();
        let rendered = crate::chatwidget::tests::render_bottom_popup(&widget, 200);
        assert!(rendered.contains("5h usage limit reached"), "{rendered}");
        assert!(rendered.contains("Ctrl+C"), "{rendered}");
        assert!(widget.auto_retry.pending.is_some());
    }
    #[tokio::test]
    async fn ctrl_c_detaches_without_cancelling_server_wait() {
        let (mut widget, _, mut events, _) = make_chatwidget_manual_with_sender().await;
        widget.on_auto_retry(notification());
        while events.try_recv().is_ok() {}
        widget.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(widget.auto_retry.pending.is_some());
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::Exit(crate::app_event::ExitMode::Immediate))
        ));
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, AppEvent::CancelAutoRetry { .. }));
        }
    }
    #[tokio::test]
    async fn ctrl_b_no_longer_detaches_and_cancels_wait_like_other_input() {
        let (mut widget, _, mut events, _) = make_chatwidget_manual_with_sender().await;
        widget.thread_id = Some(ThreadId::new());
        widget.on_auto_retry(notification());
        while events.try_recv().is_ok() {}
        widget.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        assert!(widget.auto_retry.pending.is_none());
        let mut cancelled = false;
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, AppEvent::Exit(_)));
            cancelled |= matches!(event, AppEvent::CancelAutoRetry { .. });
        }
        assert!(cancelled);
    }

    #[tokio::test]
    async fn paste_cancels_and_late_notification_cannot_restore_wait() {
        let (mut widget, _, _, _) = make_chatwidget_manual_with_sender().await;
        let status = notification();
        widget.on_auto_retry(status.clone());
        widget.handle_paste("draft".into());
        assert!(widget.auto_retry.pending.is_none());
        widget.on_auto_retry(status);
        assert!(widget.auto_retry.pending.is_none());
        assert_eq!(widget.bottom_pane.composer_text(), "draft");
    }
}
