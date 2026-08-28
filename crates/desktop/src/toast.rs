use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_base::{ToastManager, ToastMotion, ToastOptions, ToastStackState, ToastTransitionStatus};

pub(crate) const TOAST_MOTION_DURATION: Duration = Duration::from_millis(120);
pub(crate) const TOAST_AUTO_DISMISS: Duration = Duration::from_secs(4);
pub(crate) const TOAST_MAX_ACTIVE: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToastIntent {
    /// Available for confirmations that have no other visible result.
    Success,
    Info,
    Warning,
    Error,
}

impl ToastIntent {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Success => "Success",
            Self::Info => "Information",
            Self::Warning => "Warning",
            Self::Error => "Error",
        }
    }

    pub(crate) const fn timeout(self) -> Option<Duration> {
        match self {
            Self::Success | Self::Info => Some(TOAST_AUTO_DISMISS),
            Self::Warning | Self::Error => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToastMessage {
    pub(crate) intent: ToastIntent,
    pub(crate) message: String,
}

pub(crate) type ToastId = u64;

pub(crate) fn toast_motion() -> ToastMotion {
    let mut motion = ToastMotion::sonner();
    motion.duration = TOAST_MOTION_DURATION;
    motion.exit_duration = TOAST_MOTION_DURATION;
    motion
}

pub(crate) fn toast_stack_motion() -> ToastMotion {
    let mut motion = toast_motion();
    // Stack duration interpolates height and vertical offsets. Keep it instant so
    // enter/exit motion stays the per-toast horizontal slide.
    motion.duration = Duration::ZERO;
    motion
}

struct ToastTiming {
    phase_started: Instant,
    timeout_remaining: Option<Duration>,
}

pub(crate) struct ToastCenter {
    manager: ToastManager<ToastId, ToastMessage>,
    next_id: ToastId,
    last_now: Option<Instant>,
    timings: HashMap<ToastId, ToastTiming>,
    pub(crate) stack_state: ToastStackState,
}

impl Default for ToastCenter {
    fn default() -> Self {
        Self {
            manager: ToastManager::new(toast_motion()),
            next_id: 0,
            last_now: None,
            timings: HashMap::new(),
            stack_state: ToastStackState::default(),
        }
    }
}

impl ToastCenter {
    pub(crate) fn push(
        &mut self,
        intent: ToastIntent,
        message: impl Into<String>,
        now: Instant,
    ) -> ToastId {
        let active: Vec<ToastId> = self
            .iter()
            .filter(|(_, _, status)| *status != ToastTransitionStatus::Ending)
            .map(|(id, _, _)| id)
            .collect();
        if active.len() >= TOAST_MAX_ACTIVE {
            let overflow = active.len() + 1 - TOAST_MAX_ACTIVE;
            for id in active.into_iter().take(overflow) {
                self.dismiss(id, now);
            }
        }
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        self.manager.push(
            id,
            ToastMessage {
                intent,
                message: message.into(),
            },
            ToastOptions {
                timeout: intent.timeout(),
            },
            now,
        );
        self.timings.insert(
            id,
            ToastTiming {
                phase_started: now,
                timeout_remaining: intent.timeout(),
            },
        );
        self.last_now.get_or_insert(now);
        id
    }

    pub(crate) fn dismiss(&mut self, id: ToastId, now: Instant) -> bool {
        if self.manager.dismiss(&id, now) {
            if let Some(timing) = self.timings.get_mut(&id) {
                timing.phase_started = now;
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn advance(&mut self, now: Instant, paused: bool) -> bool {
        let previously_present: Vec<ToastId> = self
            .iter()
            .filter(|(_, _, status)| *status == ToastTransitionStatus::Present)
            .map(|(id, _, _)| id)
            .collect();
        let delta = self
            .last_now
            .map(|last| now.saturating_duration_since(last))
            .unwrap_or(Duration::ZERO);
        self.last_now = Some(now);
        let result = self.manager.advance(now, paused);
        if !paused {
            for id in previously_present {
                if let Some(timing) = self.timings.get_mut(&id)
                    && let Some(remaining) = timing.timeout_remaining.as_mut()
                {
                    *remaining = remaining.saturating_sub(delta);
                }
            }
        }
        for id in &result.presented {
            if let Some(timing) = self.timings.get_mut(id) {
                timing.phase_started = now;
            }
        }
        for id in &result.ending {
            if let Some(timing) = self.timings.get_mut(id) {
                timing.phase_started = now;
            }
        }
        for (id, _) in &result.removed {
            self.timings.remove(id);
        }
        result.changed
    }

    pub(crate) fn next_wake(&self, now: Instant, paused: bool) -> Option<Duration> {
        self.iter()
            .filter_map(|(id, _, status)| {
                let timing = self.timings.get(&id)?;
                match status {
                    ToastTransitionStatus::Starting | ToastTransitionStatus::Ending => Some(
                        TOAST_MOTION_DURATION
                            .saturating_sub(now.saturating_duration_since(timing.phase_started)),
                    ),
                    ToastTransitionStatus::Present if !paused => timing.timeout_remaining,
                    ToastTransitionStatus::Present => None,
                }
            })
            .min()
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.manager.is_empty()
    }

    pub(crate) fn iter(
        &self,
    ) -> impl DoubleEndedIterator<Item = (ToastId, &ToastMessage, ToastTransitionStatus)> {
        self.manager
            .iter()
            .map(|(id, message, status)| (*id, message, status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_timeouts_match_the_desktop_contract() {
        assert_eq!(ToastIntent::Success.timeout(), Some(TOAST_AUTO_DISMISS));
        assert_eq!(ToastIntent::Info.timeout(), Some(TOAST_AUTO_DISMISS));
        assert_eq!(ToastIntent::Warning.timeout(), None);
        assert_eq!(ToastIntent::Error.timeout(), None);
    }

    #[test]
    fn center_preserves_insertion_order_and_manual_dismissal_is_targeted() {
        let start = Instant::now();
        let mut center = ToastCenter::default();
        let first = center.push(ToastIntent::Info, "First", start);
        let second = center.push(ToastIntent::Warning, "Second", start);

        assert_eq!(
            center.iter().map(|(id, _, _)| id).collect::<Vec<_>>(),
            vec![first, second]
        );
        assert!(center.dismiss(first, start));
        assert_eq!(
            center
                .iter()
                .map(|(id, _, status)| (id, status))
                .collect::<Vec<_>>(),
            vec![
                (first, ToastTransitionStatus::Ending),
                (second, ToastTransitionStatus::Starting),
            ]
        );
    }

    #[test]
    fn auto_dismiss_pauses_while_the_stack_is_interactive() {
        let start = Instant::now();
        let mut center = ToastCenter::default();
        center.push(ToastIntent::Success, "Saved", start);
        center.advance(start + TOAST_MOTION_DURATION, false);

        center.advance(
            start + TOAST_MOTION_DURATION + Duration::from_secs(10),
            true,
        );
        assert_eq!(
            center.iter().next().map(|(_, _, status)| status),
            Some(ToastTransitionStatus::Present)
        );

        center.advance(
            start + TOAST_MOTION_DURATION + Duration::from_secs(14),
            false,
        );
        assert_eq!(
            center.iter().next().map(|(_, _, status)| status),
            Some(ToastTransitionStatus::Ending)
        );
    }

    #[test]
    fn next_wake_uses_timeout_boundaries_instead_of_a_frame_tick() {
        let start = Instant::now();
        let mut center = ToastCenter::default();
        center.push(ToastIntent::Success, "Saved", start);
        assert_eq!(center.next_wake(start, false), Some(TOAST_MOTION_DURATION));

        center.advance(start + TOAST_MOTION_DURATION, false);
        assert_eq!(
            center.next_wake(start + TOAST_MOTION_DURATION, false),
            Some(TOAST_AUTO_DISMISS)
        );
        assert_eq!(center.next_wake(start + TOAST_MOTION_DURATION, true), None);
    }

    #[test]
    fn overflow_dismisses_the_oldest_active_toast() {
        let start = Instant::now();
        let mut center = ToastCenter::default();
        let first = center.push(ToastIntent::Error, "One", start);
        let second = center.push(ToastIntent::Error, "Two", start);
        let third = center.push(ToastIntent::Error, "Three", start);
        let fourth = center.push(ToastIntent::Error, "Four", start);

        assert_eq!(
            center
                .iter()
                .map(|(id, _, status)| (id, status))
                .collect::<Vec<_>>(),
            vec![
                (first, ToastTransitionStatus::Ending),
                (second, ToastTransitionStatus::Starting),
                (third, ToastTransitionStatus::Starting),
                (fourth, ToastTransitionStatus::Starting),
            ]
        );
    }

    #[test]
    fn clear_drops_mounted_toasts() {
        let start = Instant::now();
        let mut center = ToastCenter::default();
        center.push(ToastIntent::Error, "Stale", start);
        center.clear();
        assert!(center.is_empty());
        assert_eq!(center.next_wake(start, false), None);
    }
}
