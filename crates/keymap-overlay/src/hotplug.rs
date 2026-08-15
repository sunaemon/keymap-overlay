//! Requesting Raw HID enumeration when a keyboard arrives.
//!
//! Existing readers stay open while the listener enumerates newly arrived
//! devices. Interrupting healthy readers would create a gap in which a layer
//! release can be lost, leaving the overlay visible indefinitely.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;

/// Coalesces an arrival burst into one enumeration request.
#[derive(Clone)]
pub(crate) struct EnumerationRequester {
    pending: Arc<AtomicBool>,
    wake: Sender<()>,
}

impl EnumerationRequester {
    pub(crate) fn new(wake: Sender<()>) -> Self {
        Self {
            pending: Arc::new(AtomicBool::new(false)),
            wake,
        }
    }

    /// Requests enumeration, returning whether this was the first pending request.
    pub(crate) fn request(&self) -> bool {
        if self.pending.swap(true, Ordering::Relaxed) {
            return false;
        }
        if self.wake.send(()).is_err() {
            self.pending.store(false, Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Lets a later arrival request another pass once this one starts.
    pub(crate) fn begin_enumeration(&self) {
        self.pending.store(false, Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::spawn_watcher;

#[cfg(target_os = "macos")]
pub(crate) use macos::spawn_watcher;

#[cfg(target_os = "macos")]
mod macos {
    use super::EnumerationRequester;
    use crate::{RAW_USAGE_ID, RAW_USAGE_PAGE};
    use anyhow::{Context, Result};
    use iohidmanager::async_api::ManagerDeviceMatchingStream;
    use iohidmanager::{HidManager, HidUsage};
    use log::{info, warn};
    use std::thread;

    const ARRIVAL_BUFFER_SIZE: usize = 16;

    pub(crate) fn spawn_watcher(requester: EnumerationRequester) {
        thread::spawn(move || {
            if let Err(error) = watch_for_arrivals(&requester) {
                // Not fatal: reader failures still request enumeration. Only a
                // later arrival alongside another healthy keyboard is missed.
                warn!("Stopped watching for keyboards: {error:#}");
            }
        });
    }

    /// Blocks on IOHIDManager callbacks, so an idle overlay costs nothing.
    fn watch_for_arrivals(requester: &EnumerationRequester) -> Result<()> {
        let manager = HidManager::new().context("Failed to create an IOHIDManager")?;
        manager
            .set_device_matching(Some(HidUsage::Custom(
                u32::from(RAW_USAGE_PAGE),
                u32::from(RAW_USAGE_ID),
            )))
            .context("Failed to match the Raw HID usage")?;

        // Registering the callback reports every device already present. Those
        // devices are part of the listener's initial enumeration, not arrivals.
        // Match identities instead of waiting for a callback count: a device
        // can disappear while the watcher starts, and that must not stall it.
        let mut existing_devices = manager.devices();
        let arrivals = ManagerDeviceMatchingStream::subscribe(&manager, ARRIVAL_BUFFER_SIZE);
        while let Some(arrival) = pollster::block_on(arrivals.next()) {
            let info = arrival.device.info();
            if let Some(index) = existing_devices
                .iter()
                .position(|existing| *existing == info)
            {
                existing_devices.swap_remove(index);
                continue;
            }
            if requester.request() {
                info!("A Raw HID device appeared; enumerating again");
            }
        }
        anyhow::bail!("The IOHIDManager arrival stream ended")
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::EnumerationRequester;
    use anyhow::{Context, Result};
    use log::{info, warn};
    use rustix::event::{PollFd, PollFlags, poll};
    use std::os::fd::AsFd;
    use std::thread;

    pub(crate) fn spawn_watcher(requester: EnumerationRequester) {
        thread::spawn(move || {
            if let Err(error) = watch_for_arrivals(&requester) {
                // Not fatal: without it, keyboards are still picked up whenever
                // one of the active readers ends.
                warn!("Stopped watching for keyboards: {error:#}");
            }
        });
    }

    /// Blocks on the udev socket forever, so an idle overlay costs nothing.
    fn watch_for_arrivals(requester: &EnumerationRequester) -> Result<()> {
        let socket = udev::MonitorBuilder::new()
            .context("Failed to open a udev monitor")?
            .match_subsystem("hidraw")
            .context("Failed to match the hidraw subsystem")?
            .listen()
            .context("Failed to listen for udev events")?;

        loop {
            wait_readable(&socket)?;
            // Events are drained in a batch: plugging in one keyboard emits
            // several, and they should cost one enumeration between them.
            // The monitor reports rules-processed events, so the uaccess ACL
            // the overlay needs is already on the node by the time this fires.
            let arrived = socket.iter().fold(false, |arrived, event| {
                arrived | (event.event_type() == udev::EventType::Add)
            });
            if arrived && requester.request() {
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

#[cfg(target_os = "windows")]
pub(crate) fn spawn_watcher(_requester: EnumerationRequester) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn an_arrival_requests_enumeration() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);

        assert!(requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
    }

    #[test]
    fn an_arrival_burst_is_coalesced() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);

        assert!(requester.request());
        assert!(!requester.request());
        assert!(!requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn a_later_arrival_can_request_another_pass() {
        let (sender, receiver) = mpsc::channel();
        let requester = EnumerationRequester::new(sender);
        requester.request();
        receiver.recv().expect("first request");

        requester.begin_enumeration();

        assert!(requester.request());
        assert_eq!(receiver.try_recv(), Ok(()));
    }
}
