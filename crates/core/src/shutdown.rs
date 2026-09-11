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

    /// Await `fut`, but give up as soon as the signal is raised: `None` means
    /// shutdown won the race and the caller should stop what it is doing.
    ///
    /// This exists for response bodies that outlive a single request — the SSE
    /// streams. A stream parked on `broadcast::Receiver::recv()` waits for a
    /// sender that the running server owns, so at shutdown it waits on a
    /// process that is waiting on it: the HTTP server's graceful shutdown does
    /// not finish until every response body ends, and that body never ends.
    /// Measured before this existed: a kernel with one `/api/events` client
    /// attached never returned from SIGTERM and was SIGKILLed by its
    /// supervisor 30s later, while the same kernel with an idle keep-alive
    /// socket — or a half-sent request — exited in 0.11s.
    ///
    /// `fut` MUST be cancel-safe: it is dropped unpolled when shutdown wins.
    /// `broadcast::Receiver::recv()`, the only caller today, documents that it
    /// is.
    pub async fn until<F: std::future::Future>(&self, fut: F) -> Option<F::Output> {
        tokio::select! {
            // Bias the shutdown arm so a raised signal ends the stream even
            // when the other future is also ready: at shutdown the events
            // still arriving are the subsystems announcing their own exit.
            biased;
            () = self.raised() => None,
            out = fut => Some(out),
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

    /// `until` must hand back the inner future's value while nothing is
    /// shutting down — otherwise the streams that use it would stop relaying.
    #[tokio::test]
    async fn until_passes_the_value_through_while_the_signal_is_down() {
        let signal = ShutdownSignal::new();
        let out = tokio::time::timeout(Duration::from_secs(5), signal.until(async { 7 }))
            .await
            .expect("until must not block when the signal is down");
        assert_eq!(out, Some(7), "the inner future's value must come through");
    }

    /// The case this helper exists for: the inner future never resolves, and
    /// only the signal can end the wait.
    #[tokio::test]
    async fn until_gives_up_on_a_future_that_never_resolves_once_the_signal_is_raised() {
        let signal = ShutdownSignal::new();
        let raiser = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            raiser.raise();
        });

        let out = tokio::time::timeout(
            Duration::from_secs(5),
            signal.until(std::future::pending::<()>()),
        )
        .await
        .expect("a raised signal must end the wait on a future that never resolves");
        assert!(out.is_none(), "shutdown winning the race must report None");
    }

    /// A signal raised before the call must still win: the streams subscribe
    /// long before shutdown, and a client that connects during shutdown must
    /// not open a body that nothing will close.
    #[tokio::test]
    async fn until_gives_up_immediately_when_the_signal_was_already_raised() {
        let signal = ShutdownSignal::new();
        signal.raise();

        let out = tokio::time::timeout(
            Duration::from_secs(5),
            signal.until(std::future::pending::<()>()),
        )
        .await
        .expect("an already-raised signal must not wait for a second raise");
        assert!(out.is_none(), "shutdown winning the race must report None");
    }
}
