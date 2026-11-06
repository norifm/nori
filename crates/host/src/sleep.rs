//! The sleep timer: the core's (`nori_core::sleep`), its time waited for by a task here.

use std::time::Duration;

use nori_core::menus::SleepChoice;
use nori_core::sleep::Sleep;
use nori_engine::Event;

use crate::session::Handle;

/// The clock the timer is set on, ms.
fn now_ms() -> i64 {
    nori_time::monotonic().as_millis() as i64
}

impl Handle {
    /// Sets the sleep timer to `choice` (all zeros: off). When its time comes the music here pauses.
    pub fn sleep_pick(&self, choice: SleepChoice) {
        let plan = self.core.session.sleep_pick(choice, now_ms());
        self.engine.pause_at_end(plan.pause_at_end);
        let Some(alarm) = plan.alarm else { return };
        let (weak, tasks) = (self.weak(), self.tasks.clone());
        self.run(async move {
            tasks.sleep(Duration::from_millis((alarm.at_ms - now_ms()).max(0) as u64)).await;
            let Some(parts) = weak.upgrade() else { return };
            // A timer set again meanwhile is not due yet.
            if parts.core.session.sleep_due(now_ms()) {
                parts.engine.pause();
            }
        });
    }

    /// The sleep timer as it runs now.
    pub fn sleep(&self) -> Sleep {
        self.core.session.sleep()
    }

    /// The music stopped by itself: a timer for that song's end is over.
    pub(crate) fn sleep_heard(&self, e: &Event) {
        if matches!(e, Event::Stopped { .. }) && !self.engine.superseded(e) {
            self.core.session.sleep_stopped();
        }
    }
}
