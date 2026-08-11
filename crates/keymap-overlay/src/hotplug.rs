//! Ending the Raw HID session when a keyboard arrives.
//!
//! A session reads from the devices it opened and ends when one of *those*
//! fails, so the listener only ever re-enumerates after a disconnect. A
//! keyboard that appears while the session is healthy — reconnected after a
//! flash, replugged, switched back by a KVM — would go unnoticed until the
//! process restarted. Ending the session is all it takes: the listener loop
//! enumerates again on its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// A handle on whichever session is currently reading, held by the watcher.
#[derive(Clone, Default)]
pub(crate) struct RunningSession(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    cancelled: Option<Arc<AtomicBool>>,
    /// A keyboard that arrived between two sessions, which would otherwise be
    /// missed: the enumeration that would have found it may already have run.
    arrived_while_detached: bool,
}

impl RunningSession {
    /// Hands the watcher the flag that stops this session's readers.
    pub(crate) fn attach(&self, cancelled: &Arc<AtomicBool>) {
        let mut state = self.state();
        if std::mem::take(&mut state.arrived_while_detached) {
            cancelled.store(true, Ordering::Relaxed);
        }
        state.cancelled = Some(Arc::clone(cancelled));
    }

    /// Forgets the session, so a later arrival is remembered rather than
    /// setting a flag nothing reads any more.
    pub(crate) fn detach(&self) {
        self.state().cancelled = None;
    }

    /// Ends the running session, or arranges for the next one to end at once.
    ///
    /// Returns whether this changed anything: one keyboard registers several
    /// `hidraw` nodes and so arrives several times, and all but the first of
    /// those find the session already on its way out.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(crate) fn end(&self) -> bool {
        let mut state = self.state();
        match &state.cancelled {
            Some(cancelled) => !cancelled.swap(true, Ordering::Relaxed),
            None => !std::mem::replace(&mut state.arrived_while_detached, true),
        }
    }

    /// A panicking session must not wedge the watcher, and there is no state
    /// here that a panic could leave inconsistent.
    fn state(&self) -> MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::spawn_watcher;

#[cfg(target_os = "linux")]
mod linux {
    use super::RunningSession;
    use anyhow::{Context, Result};
    use log::{info, warn};
    use rustix::event::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    use std::thread;

    pub(crate) fn spawn_watcher(session: RunningSession) {
        thread::spawn(move || {
            if let Err(error) = watch_for_arrivals(&session) {
                // Not fatal: without it, keyboards are still picked up whenever
                // a session ends, which is what the overlay did before.
                warn!("Stopped watching for keyboards: {error:#}");
            }
        });
    }

    /// Blocks on the udev socket forever, so an idle overlay costs nothing.
    fn watch_for_arrivals(session: &RunningSession) -> Result<()> {
        let socket = udev::MonitorBuilder::new()
            .context("Failed to open a udev monitor")?
            .match_subsystem("hidraw")
            .context("Failed to match the hidraw subsystem")?
            .listen()
            .context("Failed to listen for udev events")?;

        loop {
            wait_readable(&socket)?;
            // Events are drained in a batch: plugging in one keyboard emits
            // several, and they should cost one re-enumeration between them.
            // The monitor reports rules-processed events, so the uaccess ACL
            // the overlay needs is already on the node by the time this fires.
            let arrived = socket.iter().fold(false, |arrived, event| {
                arrived | (event.event_type() == udev::EventType::Add)
            });
            if arrived && session.end() {
                info!("A Raw HID device appeared; enumerating again");
            }
        }
    }

    fn wait_readable(socket: &udev::MonitorSocket) -> Result<()> {
        let descriptor = socket.as_fd();
        let mut fds = [PollFd::new(&descriptor, PollFlags::IN)];
        poll(&mut fds, None).context("Failed to wait on the udev monitor")?;
        Ok(())
    }
}

/// macOS has no equivalent yet: hidapi exposes no hot-plug callback and the
/// IOKit notification would be a second event source next to the run loop, so
/// there a keyboard is still picked up only once a session ends.
#[cfg(not(target_os = "linux"))]
pub(crate) fn spawn_watcher(_session: RunningSession) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    fn is_cancelled(flag: &Arc<AtomicBool>) -> bool {
        flag.load(Ordering::Relaxed)
    }

    #[test]
    fn an_arrival_ends_the_running_session() {
        let session = RunningSession::default();
        let cancelled = flag();
        session.attach(&cancelled);

        assert!(session.end());

        assert!(is_cancelled(&cancelled));
    }

    /// One keyboard arrives once per `hidraw` node it registers, and the
    /// session can only end once, so the rest have nothing left to report.
    #[test]
    fn the_rest_of_an_arrival_burst_changes_nothing() {
        let session = RunningSession::default();
        let cancelled = flag();
        session.attach(&cancelled);

        assert!(session.end());
        assert!(!session.end());
        assert!(!session.end());
    }

    #[test]
    fn a_burst_between_sessions_is_remembered_once() {
        let session = RunningSession::default();

        assert!(session.end());
        assert!(!session.end());
    }

    #[test]
    fn an_arrival_between_sessions_ends_the_next_one() {
        // The listener may have enumerated just before the keyboard appeared,
        // so the session about to start cannot be trusted to have seen it.
        let session = RunningSession::default();
        session.detach();

        session.end();

        let cancelled = flag();
        session.attach(&cancelled);
        assert!(is_cancelled(&cancelled));
    }

    #[test]
    fn a_remembered_arrival_is_only_replayed_once() {
        let session = RunningSession::default();
        session.end();

        let first = flag();
        session.attach(&first);
        session.detach();
        let second = flag();
        session.attach(&second);

        assert!(is_cancelled(&first));
        assert!(!is_cancelled(&second));
    }

    #[test]
    fn a_detached_session_is_left_alone() {
        let session = RunningSession::default();
        let cancelled = flag();
        session.attach(&cancelled);
        session.detach();

        session.end();

        assert!(!is_cancelled(&cancelled));
    }

    #[test]
    fn attaching_a_second_session_replaces_the_first() {
        let session = RunningSession::default();
        let first = flag();
        session.attach(&first);
        let second = flag();
        session.attach(&second);

        session.end();

        assert!(!is_cancelled(&first));
        assert!(is_cancelled(&second));
    }
}
