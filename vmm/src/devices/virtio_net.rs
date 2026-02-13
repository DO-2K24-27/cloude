// SPDX-License-Identifier: Apache-2.0

use std::convert::TryInto;
use std::io;
use std::io::Write;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use virtio_queue::{Queue, QueueState};
use vm_memory::{Bytes, GuestMemoryMmap};
use vmm_sys_util::eventfd::EventFd;

use super::tap::TapDevice;

pub const VIRTIO_MAGIC: u32 = 0x74726976;
pub const VIRTIO_VERSION: u32 = 2;
pub const VIRTIO_VENDOR_ID: u32 = 0;

pub const VIRTIO_NET_F_CSUM: u32 = 1 << 0;
pub const VIRTIO_NET_F_GUEST_CSUM: u32 = 1 << 1;
pub const VIRTIO_NET_F_MAC: u32 = 1 << 5;
pub const VIRTIO_NET_F_GUEST_TSO4: u32 = 1 << 7;
pub const VIRTIO_NET_F_GUEST_TSO6: u32 = 1 << 8;
pub const VIRTIO_NET_F_HOST_TSO4: u32 = 1 << 11;
pub const VIRTIO_NET_F_HOST_TSO6: u32 = 1 << 12;
pub const VIRTIO_NET_F_STATUS: u32 = 1 << 16;
pub const VIRTIO_NET_F_MRG_RXBUF: u32 = 1 << 15;

pub const VIRTIO_NET_DEVICE_FEATURES: u32 = VIRTIO_NET_F_CSUM
    | VIRTIO_NET_F_GUEST_CSUM
    | VIRTIO_NET_F_MAC
    | VIRTIO_NET_F_GUEST_TSO4
    | VIRTIO_NET_F_GUEST_TSO6
    | VIRTIO_NET_F_HOST_TSO4
    | VIRTIO_NET_F_HOST_TSO6
    | VIRTIO_NET_F_STATUS
    | VIRTIO_NET_F_MRG_RXBUF;

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
    mem: Option<GuestMemoryMmap>,
    activated: bool,
    mmio_addr: u64,

    device_features_sel: u32,
    queue_sel: u32,
    queue_size: u16,
    queue_ready: bool,
    status: u32,

    queues: Vec<Queue<Arc<GuestMemoryMmap>, QueueState>>,
}

impl VirtioNet {
    pub fn new(tap: Option<TapDevice>, mmio_addr: u64) -> io::Result<Self> {
        Ok(VirtioNet {
            device_type: 1,
            device_features: VIRTIO_NET_DEVICE_FEATURES as u64,
            config: VirtioNetConfig::new(),
            interrupt_status: Arc::new(AtomicU32::new(0)),
            interrupt_evt: EventFd::new(libc::EFD_NONBLOCK)?,
            tap,
            mem: None,
            activated: false,
            mmio_addr,
            device_features_sel: 0,
            queue_sel: 0,
            queue_size: VIRTIO_NET_QUEUE_SIZE,
            queue_ready: false,
            status: 0,
            queues: vec![],
        })
    }

    pub fn initialize_queues(&mut self, mem: Arc<GuestMemoryMmap>) {
        self.queues = vec![
            Queue::new(Arc::clone(&mem), VIRTIO_NET_QUEUE_SIZE),
            Queue::new(Arc::clone(&mem), VIRTIO_NET_QUEUE_SIZE),
        ];
    }

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

    pub fn handle_mmio_read(&self, offset: u64, data: &mut [u8]) {
        let value: u32 = match offset {
            0x000 => VIRTIO_MAGIC,
            0x004 => VIRTIO_VERSION,
            0x008 => self.device_type as u32,
            0x00c => VIRTIO_VENDOR_ID,
            0x010 => {
                if self.device_features_sel == 0 {
                    self.device_features as u32
                } else {
                    (self.device_features >> 32) as u32
                }
            }
            0x030 => self.queue_sel as u32,
            0x034 => self.queue_size as u32,
            0x044 => {
                if self.queue_ready {
                    1
                } else {
                    0
                }
            }
            0x060 => self.interrupt_status.load(Ordering::SeqCst),
            0x070 => self.status,
            0x100..=0x17f => {
                let config_offset = (offset - 0x100) as usize;
                if config_offset < std::mem::size_of::<VirtioNetConfig>() {
                    let config_bytes = unsafe {
                        std::slice::from_raw_parts(
                            &self.config as *const _ as *const u8,
                            std::mem::size_of::<VirtioNetConfig>(),
                        )
                    };
                    config_bytes[config_offset] as u32
                } else {
                    0
                }
            }
            _ => 0,
        };

        let value_bytes = value.to_le_bytes();
        let len = data.len().min(value_bytes.len());
        data[..len].copy_from_slice(&value_bytes[..len]);
    }

    pub fn handle_mmio_write(&mut self, offset: u64, data: &[u8]) {
        if data.len() != 4 {
            return;
        }

        let value = u32::from_le_bytes(data.try_into().unwrap());

        match offset {
            0x020 => {
                self.device_features_sel = value;
            }
            0x030 => {
                self.queue_sel = value;
                self.queue_size = VIRTIO_NET_QUEUE_SIZE;
                self.queue_ready = false;
            }
            0x038 => {
                self.queue_size = value as u16;
            }
            0x044 => {
                let was_ready = self.queue_ready;
                self.queue_ready = value != 0;
                if self.queue_ready && !was_ready {
                    println!("Queue {} ready!", self.queue_sel);
                }
            }
            0x050 => {
                println!("Queue {} notify!", self.queue_sel);
                if self.queue_sel as usize == TX_QUEUE {
                    self.process_tx();
                }
            }
            0x064 => {
                self.interrupt_status.fetch_and(!value, Ordering::SeqCst);
            }
            0x070 => {
                println!("STATUS VALUE={value}");
                self.status = value;
                if value & 0x4 != 0 {
                    self.activated = true;
                    println!("VirtIO net activated!");
                }
            }
            0x080 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_desc_table_address(Some(value), None);
                }
            }
            0x084 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_desc_table_address(None, Some(value));
                }
            }
            0x090 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_avail_ring_address(Some(value), None);
                }
            }
            0x094 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_avail_ring_address(None, Some(value));
                }
            }
            0x0a0 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_used_ring_address(Some(value), None);
                }
            }
            0x0a4 => {
                if let Some(queue) = self.queues.get_mut(self.queue_sel as usize) {
                    queue.set_used_ring_address(None, Some(value));
                }
            }
            _ => {}
        }
    }

    pub fn set_memory(&mut self, mem: GuestMemoryMmap) {
        self.mem = Some(mem.clone());
        self.initialize_queues(Arc::new(mem));
    }

    fn process_tx(&mut self) {
        let mem = match &self.mem {
            Some(m) => m,
            None => return,
        };

        let queue_idx = self.queue_sel as usize;
        if queue_idx != TX_QUEUE || queue_idx >= self.queues.len() {
            return;
        }

        // Collect chains first to avoid borrow issues
        let chains_data: Vec<(u16, Vec<u8>)> = {
            let queue = &mut self.queues[queue_idx];

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
                    if let Ok(read_len) = mem.read(&mut buf, addr) {
                        packet_data.extend_from_slice(&buf[..read_len]);
                    }
                }

                data.push((head_idx, packet_data));
            }
            data
        };

        // Now process the collected data
        let queue = &mut self.queues[queue_idx];

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
