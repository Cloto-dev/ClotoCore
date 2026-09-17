//! Stopping a reply while it is being produced (`POST /api/chat/{agent_id}/stop`).
//!
//! A reply is registered under the id of the message it answers from the
//! moment that message is queued for its agent, so a stop that arrives while
//! the message still waits behind the agent's previous turn is not lost.
//!
//! A stop does not act at once. The turn *arms* its registration after it has
//! stored the user's message; only an armed turn is dropped. A stop requested
//! earlier waits for the arming and acts then — so stopping never loses what
//! the user wrote, only what the agent had not finished saying.

use dashmap::DashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

struct Entry {
    agent_id: String,
    signal: Notify,
    requested: AtomicBool,
    armed: AtomicBool,
}

impl Entry {
    fn fire_if_ready(&self) {
        if self.requested.load(Ordering::SeqCst) && self.armed.load(Ordering::SeqCst) {
            // `notify_one` keeps the wake-up when nobody is waiting yet.
            self.signal.notify_one();
        }
    }
}

#[derive(Clone, Default)]
pub struct ResponseStops {
    inner: Arc<DashMap<String, Arc<Entry>>>,
}

/// Keeps a reply registered; unregisters it when dropped.
pub struct StopRegistration {
    inner: Arc<DashMap<String, Arc<Entry>>>,
    message_id: String,
    entry: Arc<Entry>,
}

impl StopRegistration {
    /// Resolves once the reply has been stopped *and* armed.
    pub async fn stopped(&self) {
        self.entry.signal.notified().await;
    }
}

impl Drop for StopRegistration {
    fn drop(&mut self) {
        // Only this registration: the same message id may have been
        // registered again (a retry) while this one was finishing.
        let entry = self.entry.clone();
        self.inner
            .remove_if(&self.message_id, |_, e| Arc::ptr_eq(e, &entry));
    }
}

impl ResponseStops {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the reply to `message_id`, being produced by `agent_id`.
    #[must_use]
    pub fn register(&self, message_id: &str, agent_id: &str) -> StopRegistration {
        let entry = Arc::new(Entry {
            agent_id: agent_id.to_string(),
            signal: Notify::new(),
            requested: AtomicBool::new(false),
            armed: AtomicBool::new(false),
        });
        self.inner.insert(message_id.to_string(), entry.clone());
        StopRegistration {
            inner: self.inner.clone(),
            message_id: message_id.to_string(),
            entry,
        }
    }

    /// The turn answering `message_id` has stored the user's message: from
    /// now on a stop may drop it (and one already requested does).
    pub fn arm(&self, message_id: &str) {
        if let Some(entry) = self.inner.get(message_id) {
            entry.armed.store(true, Ordering::SeqCst);
            entry.fire_if_ready();
        }
    }

    /// Stop the reply to `message_id`, when `agent_id` is the one producing
    /// it. Answers whether there was such a reply to stop: `false` means it
    /// had already finished (or never existed), so whatever it produced stands.
    #[must_use]
    pub fn stop(&self, agent_id: &str, message_id: &str) -> bool {
        match self.inner.get(message_id) {
            Some(entry) if entry.agent_id == agent_id => {
                entry.requested.store(true, Ordering::SeqCst);
                entry.fire_if_ready();
                true
            }
            _ => false,
        }
    }
}

/// Run `work` unless `stop` resolves first, in which case `work` is dropped at
/// the await point it had reached and `None` is answered.
pub async fn until_stopped<F: Future, S: Future<Output = ()>>(
    work: F,
    stop: S,
) -> Option<F::Output> {
    tokio::select! {
        biased;
        () = stop => None,
        out = work => Some(out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SetOnDrop(Arc<AtomicBool>);
    impl Drop for SetOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[tokio::test]
    async fn a_stop_drops_an_armed_turn_where_it_waits_and_nothing_after_it_runs() {
        let stops = ResponseStops::new();
        let reg = stops.register("m1", "agent.a");
        let (dropped, reached_the_end) = (flag(), flag());
        let (d, r, s) = (dropped.clone(), reached_the_end.clone(), stops.clone());
        let work = async move {
            let _guard = SetOnDrop(d);
            s.arm("m1");
            std::future::pending::<()>().await;
            r.store(true, Ordering::SeqCst);
        };
        let run = tokio::spawn(async move { until_stopped(work, reg.stopped()).await });
        tokio::task::yield_now().await;
        assert!(stops.stop("agent.a", "m1"));
        assert!(run.await.unwrap().is_none());
        assert!(
            dropped.load(Ordering::SeqCst),
            "the stopped work was not dropped"
        );
        assert!(!reached_the_end.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_stop_requested_while_queued_waits_until_the_users_message_is_stored() {
        let stops = ResponseStops::new();
        let reg = stops.register("m1", "agent.a");
        // Queued behind the agent's previous turn: the stop comes first.
        assert!(stops.stop("agent.a", "m1"));
        let (stored, answered) = (flag(), flag());
        let (st, an, s) = (stored.clone(), answered.clone(), stops.clone());
        let work = async move {
            tokio::task::yield_now().await; // the store is asynchronous
            st.store(true, Ordering::SeqCst);
            s.arm("m1");
            tokio::task::yield_now().await; // the engine call
            an.store(true, Ordering::SeqCst);
        };
        assert!(until_stopped(work, reg.stopped()).await.is_none());
        assert!(
            stored.load(Ordering::SeqCst),
            "the stop dropped the turn before it stored the message"
        );
        assert!(
            !answered.load(Ordering::SeqCst),
            "the stop did not act once armed"
        );
    }

    #[tokio::test]
    async fn a_turn_that_never_arms_is_never_dropped() {
        let stops = ResponseStops::new();
        let reg = stops.register("m1", "agent.a");
        assert!(stops.stop("agent.a", "m1"));
        let out = until_stopped(
            async {
                tokio::task::yield_now().await;
                7
            },
            reg.stopped(),
        )
        .await;
        assert_eq!(out, Some(7));
    }

    #[tokio::test]
    async fn an_armed_turn_nobody_stops_answers_its_output() {
        let stops = ResponseStops::new();
        let reg = stops.register("m1", "agent.a");
        stops.arm("m1");
        assert_eq!(until_stopped(async { 7 }, reg.stopped()).await, Some(7));
    }

    #[test]
    fn only_the_agent_producing_the_reply_can_stop_it() {
        let stops = ResponseStops::new();
        let _reg = stops.register("m1", "agent.a");
        assert!(!stops.stop("agent.b", "m1"));
        assert!(!stops.stop("agent.a", "m2"));
        assert!(stops.stop("agent.a", "m1"));
    }

    #[test]
    fn a_finished_reply_cannot_be_stopped_and_a_newer_registration_survives_an_older_one() {
        let stops = ResponseStops::new();
        let older = stops.register("m1", "agent.a");
        let newer = stops.register("m1", "agent.a");
        drop(older);
        assert!(
            stops.stop("agent.a", "m1"),
            "the older registration removed the newer one"
        );
        drop(newer);
        assert!(!stops.stop("agent.a", "m1"));
    }
}
