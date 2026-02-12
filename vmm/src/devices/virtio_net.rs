// SPDX-License-Identifier: Apache-2.0

use std::convert::TryInto;
use std::io::{self, Result};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use vm_memory::{Address, GuestMemoryMmap};
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
    queue_desc: u64,
    queue_avail: u64,
    queue_used: u64,

    tx_last_avail: u16,
    rx_last_avail: u16,

    kill_evt: EventFd,
}

impl VirtioNet {
    pub fn new(tap: Option<TapDevice>, mmio_addr: u64) -> Result<Self> {
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
            queue_desc: 0,
            queue_avail: 0,
            queue_used: 0,
            tx_last_avail: 0,
            rx_last_avail: 0,
            kill_evt: EventFd::new(libc::EFD_NONBLOCK)?,
        })
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
        0x200
    }

    pub fn cmdline_string(&self, irq: u32) -> String {
        format!(" virtio_mmio.device=4K@{:#x}:{}", self.mmio_addr, irq)
    }

    pub fn handle_mmio_read(&self, offset: u64, data: &mut [u8]) {
        if data.len() != 4 {
            return;
        }

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

        data.copy_from_slice(&value.to_le_bytes());
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
                    println!(
                        "Queue {} ready! desc={:#x}",
                        self.queue_sel, self.queue_desc
                    );
                }
            }
            0x050 => {
                println!("Queue {} notify!", self.queue_sel);
            }
            0x064 => {
                self.interrupt_status.fetch_and(!value, Ordering::SeqCst);
            }
            0x070 => {
                self.status = value;
                if value & 0x4 != 0 {
                    self.activated = true;
                    println!("VirtIO net activated!");
                }
            }
            0x080 => {
                self.queue_desc = (self.queue_desc & !0xFFFFFFFF) | (value as u64);
            }
            0x084 => {
                self.queue_desc = (self.queue_desc & 0xFFFFFFFF) | ((value as u64) << 32);
            }
            0x090 => {
                self.queue_avail = (self.queue_avail & !0xFFFFFFFF) | (value as u64);
            }
            0x094 => {
                self.queue_avail = (self.queue_avail & 0xFFFFFFFF) | ((value as u64) << 32);
            }
            0x0a0 => {
                self.queue_used = (self.queue_used & !0xFFFFFFFF) | (value as u64);
            }
            0x0a4 => {
                self.queue_used = (self.queue_used & 0xFFFFFFFF) | ((value as u64) << 32);
            }
            _ => {}
        }
    }

    pub fn set_memory(&mut self, mem: GuestMemoryMmap) {
        self.mem = Some(mem);
    }

    pub fn signal_interrupt(&self) {
        self.interrupt_status.fetch_or(0x1, Ordering::SeqCst);
        let _ = self.interrupt_evt.write(1);
    }

    pub fn handle_interrupt(&mut self) -> Result<()> {
        self.interrupt_status.fetch_and(!0x1, Ordering::SeqCst);
        Ok(())
    }

    pub fn set_mac(&mut self, mac: [u8; 6]) {
        self.config.mac = mac;
    }
}
