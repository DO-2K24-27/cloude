// SPDX-License-Identifier: Apache-2.0

use std::borrow::{Borrow, BorrowMut};
use std::convert::{TryFrom, TryInto};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use event_manager::{MutEventSubscriber, RemoteEndpoint, SubscriberId};
use kvm_ioctls::{IoEventAddress, VmFd};
use libc::EFD_NONBLOCK;
use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioDeviceType, VirtioMmioDevice};
use virtio_queue::Queue;
use vm_allocator::RangeInclusive;
use vm_device::bus::MmioAddress;
use vm_device::MutDeviceMmio;
use vm_memory::{GuestMemoryMmap, GuestUsize};
use vmm_sys_util::eventfd::EventFd;

use crate::devices::virtio::net::queue_handler::QueueHandler;
use crate::devices::virtio::net::simple_handler::SimpleHandler;
use crate::devices::virtio::net::tap::Tap;
use crate::devices::virtio::net::VIRTIO_NET_HDR_SIZE;
use crate::devices::virtio::{Error, SingleFdSignalQueue, VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET};

pub const VIRTIO_F_RING_EVENT_IDX: u64 = 29;
pub const VIRTIO_F_VERSION_1: u64 = 32;
pub const VIRTIO_F_IN_ORDER: u64 = 35;

pub const VIRTIO_NET_F_CSUM: u64 = 0;
pub const VIRTIO_NET_F_GUEST_CSUM: u64 = 1;
pub const VIRTIO_NET_F_GUEST_TSO4: u64 = 7;
pub const VIRTIO_NET_F_GUEST_TSO6: u64 = 8;
pub const VIRTIO_NET_F_GUEST_UFO: u64 = 10;
pub const VIRTIO_NET_F_HOST_TSO4: u64 = 11;
pub const VIRTIO_NET_F_HOST_TSO6: u64 = 12;
pub const VIRTIO_NET_F_HOST_UFO: u64 = 14;

pub const VIRTIO_NET_DEVICE_FEATURES: u64 = (1 << VIRTIO_F_VERSION_1)
    | (1 << VIRTIO_F_RING_EVENT_IDX)
    | (1 << VIRTIO_F_IN_ORDER)
    | (1 << VIRTIO_NET_F_CSUM)
    | (1 << VIRTIO_NET_F_GUEST_CSUM)
    | (1 << VIRTIO_NET_F_GUEST_TSO4)
    | (1 << VIRTIO_NET_F_GUEST_TSO6)
    | (1 << VIRTIO_NET_F_GUEST_UFO)
    | (1 << VIRTIO_NET_F_HOST_TSO4)
    | (1 << VIRTIO_NET_F_HOST_TSO6)
    | (1 << VIRTIO_NET_F_HOST_UFO);

pub const VIRTIO_NET_QUEUE_SIZE: u16 = 256;

pub const TUN_F_CSUM: ::std::os::raw::c_uint = 1;
pub const TUN_F_TSO4: ::std::os::raw::c_uint = 2;
pub const TUN_F_TSO6: ::std::os::raw::c_uint = 4;
pub const TUN_F_UFO: ::std::os::raw::c_uint = 16;

pub struct VirtioNetDevice {
    vm_fd: Arc<VmFd>,
    guest_memory: Arc<GuestMemoryMmap>,
    tap_name: String,
    /// addresses where the device lives in the guest
    pub mmio_range: RangeInclusive,
    // IRQ (id on the guest side), for signaling the driver (guest)
    irq: u32,
    /// IRQ eventfd (id on the VMM side) for signaling the driver (guest).
    irqfd: Arc<EventFd>,
    /// virtio device config sur lib
    virtio_cfg: VirtioConfig<Arc<GuestMemoryMmap>>,
    /// handler for tx/rx/tap events
    pub handler: Option<Arc<Mutex<QueueHandler<Arc<GuestMemoryMmap>>>>>,
    endpoint: RemoteEndpoint<Subscriber>,
}

type Subscriber = Arc<Mutex<dyn MutEventSubscriber>>;

impl VirtioNetDevice {
    pub fn new(
        vm_fd: Arc<VmFd>,
        irq: u32,
        tap_name: String,
        guest_memory: Arc<GuestMemoryMmap>,
        mmio_range: RangeInclusive,
        endpoint: RemoteEndpoint<Subscriber>,
    ) -> Result<Self, Error> {
        let queues = vec![
            Queue::new(guest_memory.clone(), VIRTIO_NET_QUEUE_SIZE),
            Queue::new(guest_memory.clone(), VIRTIO_NET_QUEUE_SIZE),
        ];

        let irqfd = Arc::new(EventFd::new(EFD_NONBLOCK).map_err(Error::Io)?);
        vm_fd
            .register_irqfd(&irqfd, irq)
            .map_err(Error::RegisterIrqfd)?;

        let virtio_cfg = VirtioConfig::new(VIRTIO_NET_DEVICE_FEATURES as u64, queues, Vec::new());

        Ok(VirtioNetDevice {
            vm_fd,
            guest_memory,
            irq,
            irqfd,
            tap_name,
            mmio_range,
            virtio_cfg,
            handler: None,
            endpoint,
        })
    }
    // Converts a `GuestUsize` to a concise string representation, with multiplier suffixes.
    fn guestusize_to_str(size: GuestUsize) -> String {
        const KB_MULT: u64 = 1 << 10;
        const MB_MULT: u64 = KB_MULT << 10;
        const GB_MULT: u64 = MB_MULT << 10;

        if size % GB_MULT == 0 {
            return format!("{}G", size / GB_MULT);
        }
        if size % MB_MULT == 0 {
            return format!("{}M", size / MB_MULT);
        }
        if size % KB_MULT == 0 {
            return format!("{}K", size / KB_MULT);
        }
        size.to_string()
    }

    pub fn cmdline_string(&self) -> String {
        format!(
            " virtio_mmio.device={}@{:#x}:{}",
            Self::guestusize_to_str(self.mmio_range.len()),
            self.mmio_range.start(),
            self.irq
        )
    }
}

type MyVirtioConfig = VirtioConfig<Arc<GuestMemoryMmap>>;

impl VirtioDeviceType for VirtioNetDevice {
    fn device_type(&self) -> u32 {
        1 // NET_DEVICE_ID
    }
}

impl Borrow<MyVirtioConfig> for VirtioNetDevice {
    fn borrow(&self) -> &MyVirtioConfig {
        &self.virtio_cfg
    }
}

impl BorrowMut<MyVirtioConfig> for VirtioNetDevice {
    fn borrow_mut(&mut self) -> &mut MyVirtioConfig {
        &mut self.virtio_cfg
    }
}

impl VirtioNetDevice {
    fn setup_tap(&mut self) -> Result<Tap, Error> {
        let tap = Tap::open_named(self.tap_name.as_str()).map_err(Error::Tap)?;

        // Set offload flags to match the relevant virtio features of the device (for now,
        // statically set in the constructor.
        tap.set_offload(TUN_F_CSUM | TUN_F_UFO | TUN_F_TSO4 | TUN_F_TSO6)
            .map_err(Error::Tap)?;

        // The layout of the header is specified in the standard and is 12 bytes in size. We
        // should define this somewhere.
        tap.set_vnet_hdr_size(VIRTIO_NET_HDR_SIZE as i32)
            .map_err(Error::Tap)?;

        Ok(tap)
    }

    fn setup_handler(
        &mut self,
        tap: Tap,
        queue_eventfds: [EventFd; 2],
    ) -> Result<QueueHandler<Arc<GuestMemoryMmap>>, Error> {
        // Setup driver (guest) notification
        let driver_notify = SingleFdSignalQueue {
            irqfd: self.irqfd.clone(),
            interrupt_status: self.virtio_cfg.interrupt_status.clone(),
        };

        let [rx_ioevent, tx_ioevent] = queue_eventfds;

        // Create handler
        let rxq = self.virtio_cfg.queues.remove(0);
        let txq = self.virtio_cfg.queues.remove(0);
        let inner = SimpleHandler::new(driver_notify, rxq, txq, tap);
        let handler = QueueHandler {
            inner,
            rx_ioevent,
            tx_ioevent,
        };

        Ok(handler)
    }

    fn register_handler(&mut self, handler: Arc<Mutex<QueueHandler<Arc<GuestMemoryMmap>>>>) {
        self.endpoint
            .call_blocking(|mgr| -> event_manager::Result<SubscriberId> {
                Ok(mgr.add_subscriber(handler))
            })
            .unwrap();
    }

    fn register_queue_events(&self) -> Result<Vec<EventFd>, Error> {
        let mut ioevents = Vec::new();

        for i in 0..self.virtio_cfg.queues.len() {
            let fd = EventFd::new(EFD_NONBLOCK).map_err(Error::Io)?;

            // Register the queue event fd.
            self.vm_fd
                .register_ioevent(
                    &fd,
                    &IoEventAddress::Mmio(
                        self.mmio_range.start() + VIRTIO_MMIO_QUEUE_NOTIFY_OFFSET,
                    ),
                    // The maximum number of queues should fit within an `u16` according to the
                    // standard, so the conversion below is always expected to succeed.
                    u32::try_from(i).unwrap(),
                )
                .map_err(Error::Kvm)?;

            ioevents.push(fd);
        }

        Ok(ioevents)
    }
}

impl VirtioDeviceActions for VirtioNetDevice {
    type E = Error;

    fn activate(&mut self) -> Result<(), Error> {
        let tap = self.setup_tap()?;

        let queue_eventfds = self.register_queue_events()?;
        let handler = self.setup_handler(
            tap,
            queue_eventfds.try_into().expect("There should be 2 queues"),
        )?;
        let handler = Arc::new(Mutex::new(handler));
        self.handler = Some(handler.clone());

        self.register_handler(handler);

        Ok(())
    }

    fn reset(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

impl VirtioMmioDevice<Arc<GuestMemoryMmap>> for VirtioNetDevice {
    fn queue_notify(&mut self, _val: u32) {
        println!("Queue notify called");
    }
}

impl MutDeviceMmio for VirtioNetDevice {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
