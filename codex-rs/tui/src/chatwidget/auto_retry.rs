use super::*;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct AutoRetry {
    pub reset: Option<i64>,
    pub goal: bool,
    pub(super) pending: Option<(Instant, bool, bool, Duration)>,
    last: Option<(Instant, Duration)>,
    ceiling: f64,
}

impl ChatWidget {
    pub(super) fn cancel_auto_retry(&mut self) {
        if self.auto_retry.pending.take().is_some() {
            self.bottom_pane
                .set_placeholder_text(self.normal_placeholder_text.clone());
            self.auto_retry.last = None;
            self.request_redraw();
        }
    }

    pub(super) fn schedule_auto_retry(&mut self, message: &str) {
        if !self.bottom_pane.composer_is_empty() {
            return;
        }
        let now = Instant::now();
        let message = message.to_ascii_lowercase();
        let capability =
            message.contains("model capability") || message.contains("model_capability");
        let duration = if capability {
            let Some(ceiling) = next_ceiling(self.auto_retry.last, self.auto_retry.ceiling, now)
            else {
                return;
            };
            self.auto_retry.ceiling = ceiling;
            Duration::from_secs_f64(rand::random::<f64>() * self.auto_retry.ceiling)
        } else if message.contains("usage limit") || message.contains("5h") {
            let Some(reset) = self.auto_retry.reset.or_else(|| parse_reset(&message)) else {
                return;
            };
            Duration::from_secs((reset - chrono::Utc::now().timestamp()).max(0) as u64)
        } else {
            return;
        };
        self.auto_retry.pending =
            Some((now + duration, self.auto_retry.goal, capability, duration));
        self.request_redraw();
    }

    pub(super) fn tick_auto_retry(&mut self) {
        let Some((deadline, goal, capability, duration)) = self.auto_retry.pending else {
            return;
        };
        let now = Instant::now();
        if now >= deadline {
            self.auto_retry.pending = None;
            self.bottom_pane
                .set_placeholder_text(self.normal_placeholder_text.clone());
            if capability {
                self.auto_retry.last = Some((now, duration + now.duration_since(deadline)));
            }
            self.input_queue.rate_limit_recovery_pending = false;
            if goal {
                self.handle_slash_command_with_args_dispatch(
                    SlashCommand::Goal,
                    "resume".into(),
                    vec![],
                );
            } else {
                self.submit_user_message("Continue".to_string().into());
            }
            return;
        }
        let remaining = deadline.duration_since(now);
        let seconds = remaining.as_secs() + 1;
        let reason = if capability {
            "Model capability error"
        } else {
            "5h usage limit reached"
        };
        self.bottom_pane.set_placeholder_text(format!(
            "{reason}. Automatically continuing in {} h {} m {} s. Type to cancel.",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60
        ));
        self.frame_requester
            .schedule_frame_in(remaining.min(Duration::from_millis(100)));
    }
}

fn next_ceiling(last: Option<(Instant, Duration)>, ceiling: f64, now: Instant) -> Option<f64> {
    match last.filter(|(end, _)| now.duration_since(*end) <= Duration::from_secs(64)) {
        Some((_, wait)) if wait > Duration::from_secs(64) => None,
        Some(_) => Some(ceiling * 2.0),
        None => Some(0.5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn paste_cancels_pending_retry() {
        let (mut widget, _sender, _events, _operations) =
            super::super::tests::make_chatwidget_manual_with_sender().await;
        widget.schedule_auto_retry("model capability error");
        assert!(widget.auto_retry.pending.is_some());
        widget.handle_paste("user input".into());
        assert!(widget.auto_retry.pending.is_none());
        assert_eq!(widget.bottom_pane.composer_text(), "user input");
    }

    #[test]
    fn reset_timestamp_parser() {
        assert!(parse_reset("try again at oct 28th, 2026 11:30 pm.").is_some());
        assert!(parse_reset("try again at 11:30 pm.").is_some());
        assert_eq!(parse_reset("try again later."), None);
    }

    #[test]
    fn backoff_boundaries() {
        let now = Instant::now();
        assert_eq!(next_ceiling(None, 99.0, now), Some(0.5));
        assert_eq!(
            next_ceiling(Some((now, Duration::from_secs(64))), 128.0, now),
            Some(256.0)
        );
        assert_eq!(
            next_ceiling(Some((now, Duration::from_secs(65))), 128.0, now),
            None
        );
        let last = Some((now, Duration::from_secs(65)));
        assert_eq!(
            next_ceiling(last, 128.0, now + Duration::from_secs(64)),
            None
        );
        assert_eq!(
            next_ceiling(last, 128.0, now + Duration::from_secs(65)),
            Some(0.5)
        );
    }
}

// The server renders reset times in the local timezone, with a date only when
// the reset is not today. Prefer the exact snapshot timestamp when available.
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
