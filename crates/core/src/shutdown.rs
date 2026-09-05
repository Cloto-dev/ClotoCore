//! Shutdown signalling for the kernel's background tasks.
//!
//! One [`ShutdownSignal`] is created at boot and cloned into every long-lived
//! task; raising it once stops all of them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// A shutdown signal that stays raised once it has been raised.
///
/// `Notify::notify_waiters()` wakes only the tasks already parked on
/// `notified()`. A task between two `select!` arms — a health monitor inside
/// its `interval.tick()`, say — misses the wake entirely and keeps running
/// until its next poll. Latching the signal in a flag alongside the notify
/// makes it observable after the fact, so a late waiter returns immediately
/// instead of waiting for a second signal that never comes.
#[derive(Clone, Default)]
pub struct ShutdownSignal {
    raised: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl ShutdownSignal {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Raise the signal. Idempotent.
    pub fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    #[must_use]
    pub fn is_raised(&self) -> bool {
        self.raised.load(Ordering::SeqCst)
    }

    /// Resolve once the signal has been raised — including when it was raised
    /// before this call.
    pub async fn raised(&self) {
        loop {
            // Create the `notified()` future BEFORE re-reading the flag: it
            // enqueues at creation, so a raise landing between the check and
            // the await is still delivered to this future rather than lost.
            let pending = self.notify.notified();
            if self.is_raised() {
                return;
            }
            pending.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ShutdownSignal;
    use std::time::Duration;

    /// The ordinary case: a task already waiting is released by `raise`.
    #[tokio::test]
    async fn a_waiter_is_released_when_the_signal_is_raised() {
        let signal = ShutdownSignal::new();
        let waiter = signal.clone();
        let task = tokio::spawn(async move { waiter.raised().await });

        // Let the task reach its await point before signalling.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!signal.is_raised(), "nothing has raised the signal yet");
        signal.raise();

        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("a waiter parked before the raise must be woken by it")
            .expect("the waiting task panicked");
    }

    /// The case a bare `Notify` loses: the raise happens first, and the waiter
    /// arrives afterwards. With `notify_waiters` alone this would hang, because
    /// the wake is delivered only to tasks already parked.
    #[tokio::test]
    async fn a_waiter_arriving_after_the_raise_resolves_immediately() {
        let signal = ShutdownSignal::new();
        signal.raise();
        assert!(signal.is_raised(), "raise must latch");

        tokio::time::timeout(Duration::from_secs(5), signal.raised())
            .await
            .expect("a waiter arriving after the raise must not block on a second signal");
    }
}
