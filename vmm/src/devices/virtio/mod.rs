use std::{
    io,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc,
    },
};

use vmm_sys_util::eventfd::EventFd;

use crate::devices::virtio::net::tap;

pub mod net;

#[derive(Debug)]
pub enum Error {
    Kvm(kvm_ioctls::Error),
    Io(io::Error),
    RegisterIrqfd(kvm_ioctls::Error),
    Tap(tap::Error),
}

// This bit is set on the device interrupt status when notifying the driver about used
// queue events.
// TODO: There seem to be similar semantics when the PCI transport is used with MSI-X cap
// disabled. Let's figure out at some point if having MMIO as part of the name is necessary.
const VIRTIO_MMIO_INT_VRING: u8 = 0x01;

// The driver will write to the register at this offset in the MMIO region to notify the device
// about available queue events.
const VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET: u64 = 0x50;

/// Simple trait to model the operation of signalling the driver about used events
/// for the specified queue.
// TODO: Does this need renaming to be relevant for packed queues as well?
pub trait SignalUsedQueue {
    // TODO: Should this return an error? This failing is not really recoverable at the interface
    // level so the expectation is the implementation handles that transparently somehow.
    fn signal_used_queue(&self, index: u16);
}

/// Uses a single irqfd as the basis of signalling any queue (useful for the MMIO transport,
/// where a single interrupt is shared for everything).
pub struct SingleFdSignalQueue {
    pub irqfd: Arc<EventFd>,
    pub interrupt_status: Arc<AtomicU8>,
}

impl SignalUsedQueue for SingleFdSignalQueue {
    fn signal_used_queue(&self, _index: u16) {
        self.interrupt_status
            .fetch_or(VIRTIO_MMIO_INT_VRING, Ordering::SeqCst);
        self.irqfd
            .write(1)
            .expect("Failed write to eventfd when signalling queue");
    }
}
