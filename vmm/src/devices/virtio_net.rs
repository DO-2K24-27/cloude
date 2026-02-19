// SPDX-License-Identifier: Apache-2.0

use std::borrow::{Borrow, BorrowMut};
use std::convert::TryInto;
use std::io;
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use virtio_device::{VirtioConfig, VirtioDeviceActions, VirtioDeviceType, VirtioMmioDevice};
use virtio_queue::{Queue, QueueState};
use vm_device::bus::MmioAddress;
use vm_device::MutDeviceMmio;
use vm_memory::{Bytes, GuestAddress, GuestAddressSpace, GuestMemory, GuestMemoryMmap};
use vmm_sys_util::eventfd::EventFd;

use super::tap::TapDevice;

pub const VIRTIO_MAGIC: u32 = 0x74726976;
pub const VIRTIO_VERSION: u32 = 2;
pub const VIRTIO_VENDOR_ID: u32 = 0;

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

pub const RX_QUEUE: usize = 0;
pub const TX_QUEUE: usize = 1;

#[derive(Copy, Clone)]
pub struct VirtioNetConfig {
    pub mac: [u8; 6],
    pub status: u16,
    pub max_virtqueue_pairs: u16,
    pub mtu: u16,
}

impl VirtioNetConfig {
    pub fn new() -> Self {
        VirtioNetConfig {
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            status: 1,
            max_virtqueue_pairs: 1,
            mtu: 1500,
        }
    }
}

pub struct VirtioNet {
    device_type: u32,
    device_features: u64,
    config: VirtioNetConfig,
    interrupt_status: Arc<AtomicU32>,
    interrupt_evt: EventFd,
    tap: Option<TapDevice>,
    mem: Arc<GuestMemoryMmap>,
    activated: bool,
    mmio_addr: u64,

    device_features_sel: u32,
    queue_size: u16,
    queue_ready: bool,
    status: u32,

    virtio_cfg: VirtioConfig<Arc<GuestMemoryMmap>>,
}

impl VirtioNet {
    pub fn new(
        tap: Option<TapDevice>,
        guest_memory: Arc<GuestMemoryMmap>,
        mmio_addr: u64,
    ) -> io::Result<Self> {
        let queues = vec![
            Queue::new(guest_memory.clone(), VIRTIO_NET_QUEUE_SIZE),
            Queue::new(guest_memory.clone(), VIRTIO_NET_QUEUE_SIZE),
        ];
        let config = VirtioConfig::new(VIRTIO_NET_DEVICE_FEATURES as u64, queues, Vec::new());
        Ok(VirtioNet {
            device_type: 1,
            device_features: VIRTIO_NET_DEVICE_FEATURES as u64,
            config: VirtioNetConfig::new(),
            interrupt_status: Arc::new(AtomicU32::new(0)),
            interrupt_evt: EventFd::new(libc::EFD_NONBLOCK)?,
            tap,
            mem: guest_memory,
            activated: false,
            mmio_addr,
            device_features_sel: 0,
            queue_size: VIRTIO_NET_QUEUE_SIZE,
            queue_ready: false,
            status: 0,
            virtio_cfg: config,
        })
    }

    pub fn initialize_queues(&mut self, mem: Arc<GuestMemoryMmap>) {}

    pub fn mmio_addr(&self) -> u64 {
        self.mmio_addr
    }

    pub fn interrupt_evt(&self) -> &EventFd {
        &self.interrupt_evt
    }

    pub fn interrupt_evt_fd(&self) -> RawFd {
        self.interrupt_evt.as_raw_fd()
    }

    pub fn interrupt_status(&self) -> &Arc<AtomicU32> {
        &self.interrupt_status
    }

    pub fn is_activated(&self) -> bool {
        self.activated && self.queue_ready
    }

    pub fn device_type(&self) -> u32 {
        self.device_type
    }

    pub fn mmio_size(&self) -> u64 {
        4096
    }

    pub fn cmdline_string(&self, irq: u32) -> String {
        format!(" virtio_mmio.device=4K@{:#x}:{}", self.mmio_addr, irq)
    }

    fn process_tx(&mut self) {
        println!("process_tx() 1");

        // Collect chains first to avoid borrow issues
        let chains_data: Vec<(u16, Vec<u8>)> = {
            let queue = &mut self.virtio_cfg.queues[TX_QUEUE];

            let mut iter = match queue.iter() {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("Error getting queue iter: {:?}", e);
                    return;
                }
            };

            let mut data = Vec::new();
            while let Some(chain) = iter.next() {
                let head_idx = chain.head_index();

                let writable_chain = chain.writable();

                let mut packet_data = Vec::new();
                for desc in writable_chain {
                    let addr = desc.addr();
                    let len = desc.len() as usize;
                    let mut buf = vec![0u8; len];
                    if let Ok(read_len) = self.mem.read(&mut buf, addr) {
                        packet_data.extend_from_slice(&buf[..read_len]);
                    }
                }

                data.push((head_idx, packet_data));
            }
            data
        };
        println!("process_tx() 2");

        // Now process the collected data
        let queue = &mut self.virtio_cfg.queues[TX_QUEUE];

        for (head_idx, packet_data) in chains_data {
            if !packet_data.is_empty() {
                println!("TX packet: {} bytes", packet_data.len());

                if let Some(ref mut tap) = self.tap {
                    if let Err(e) = tap.write(&packet_data) {
                        eprintln!("TAP write error: {:?}", e);
                    } else {
                        println!("Wrote to TAP");
                    }
                }
            }

            let len = packet_data.len() as u32;
            if let Err(e) = queue.add_used(head_idx, len) {
                eprintln!("Error adding used: {:?}", e);
            }
        }
        println!("process_tx() 3");

        self.signal_interrupt();
    }

    pub fn signal_interrupt(&self) {
        self.interrupt_status.fetch_or(0x1, Ordering::SeqCst);
        let _ = self.interrupt_evt.write(1);
    }

    pub fn handle_interrupt(&mut self) -> io::Result<()> {
        self.interrupt_status.fetch_and(!0x1, Ordering::SeqCst);
        Ok(())
    }

    pub fn set_mac(&mut self, mac: [u8; 6]) {
        self.config.mac = mac;
    }
}

type MyVirtioConfig = VirtioConfig<Arc<GuestMemoryMmap>>;

impl VirtioDeviceType for VirtioNet {
    fn device_type(&self) -> u32 {
        1 // NET_DEVICE_ID
    }
}

impl Borrow<MyVirtioConfig> for VirtioNet {
    fn borrow(&self) -> &MyVirtioConfig {
        &self.virtio_cfg
    }
}

impl BorrowMut<MyVirtioConfig> for VirtioNet {
    fn borrow_mut(&mut self) -> &mut MyVirtioConfig {
        &mut self.virtio_cfg
    }
}

impl VirtioDeviceActions for VirtioNet {
    type E = ();

    fn activate(&mut self) -> Result<(), ()> {
        Ok(())
    }

    fn reset(&mut self) -> std::result::Result<(), ()> {
        // Not implemented for now.
        Ok(())
    }
}

impl VirtioMmioDevice<Arc<GuestMemoryMmap>> for VirtioNet {
    fn queue_notify(&mut self, _val: u32) {
        println!("Queue notify called");
        self.process_tx();
    }
}

impl MutDeviceMmio for VirtioNet {
    fn mmio_read(&mut self, _base: MmioAddress, offset: u64, data: &mut [u8]) {
        let mmio_addr = self.mem.get_host_address(GuestAddress(0)).unwrap();
        println!("OK !! {:?}", mmio_addr);
        self.read(offset, data);
    }

    fn mmio_write(&mut self, _base: MmioAddress, offset: u64, data: &[u8]) {
        self.write(offset, data);
    }
}
