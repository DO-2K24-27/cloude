// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

#![cfg(target_arch = "x86_64")]

extern crate libc;

extern crate linux_loader;
extern crate vm_memory;
extern crate vm_superio;

use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};
use std::thread;
use std::{io, path::PathBuf};

use kvm_bindings::{kvm_userspace_memory_region, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VmFd};
use linux_loader::loader::{self, KernelLoaderResult};
use vm_allocator::{AddressAllocator, AllocPolicy, RangeInclusive};
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};
mod cpu;
use cpu::{cpuid, mptable, Vcpu};
mod devices;
use devices::serial::LumperSerial;
use vmm_sys_util::poll::{EpollContext, EpollEvents};

use crate::devices::tap::TapDevice;
use crate::devices::virtio_net::VirtioNet;

mod kernel;

#[cfg(target_arch = "x86_64")]
pub(crate) const MMIO_GAP_END: u64 = 1 << 32;
/// Size of the MMIO gap.
#[cfg(target_arch = "x86_64")]
pub(crate) const MMIO_GAP_SIZE: u64 = 768 << 20;
/// The start of the MMIO gap (memory area reserved for MMIO devices).
#[cfg(target_arch = "x86_64")]
pub(crate) const MMIO_GAP_START: u64 = MMIO_GAP_END - MMIO_GAP_SIZE;

#[derive(Debug)]

/// VMM errors.
pub enum Error {
    /// Failed to write boot parameters to guest memory.
    BootConfigure(linux_loader::configurator::Error),
    /// Error configuring the kernel command line.
    Cmdline(linux_loader::cmdline::Error),
    /// Failed to load kernel.
    KernelLoad(loader::Error),
    /// Invalid E820 configuration.
    E820Configuration,
    /// Highmem start address is past the guest memory end.
    HimemStartPastMemEnd,
    /// I/O error.
    IO(io::Error),
    /// Error issuing an ioctl to KVM.
    KvmIoctl(kvm_ioctls::Error),
    /// vCPU errors.
    Vcpu(cpu::Error),
    /// Memory error.
    Memory(vm_memory::Error),
    /// Serial creation error
    SerialCreation(io::Error),
    /// IRQ registration error
    IrqRegister(io::Error),
    /// Terminal configuration error
    TerminalConfigure(kvm_ioctls::Error),
    /// epoll creation error
    EpollError(io::Error),
    /// STDIN read error
    StdinRead(io::Error),
    /// STDIN write error
    StdinWrite(vm_superio::serial::Error<io::Error>),
    /// VirtIO net creation error
    VirtioNetCreation(io::Error),
    /// Address allocation error
    AddressAllocation(vm_allocator::Error),
}

/// Dedicated [`Result`](https://doc.rust-lang.org/std/result/) type.
pub type Result<T> = std::result::Result<T, Error>;

pub struct VMM {
    vm_fd: VmFd,
    kvm: Kvm,
    guest_memory: Arc<GuestMemoryMmap>,
    vcpus: Vec<Vcpu>,
    serial: Arc<Mutex<LumperSerial>>,
    virtio_net: Option<Arc<Mutex<VirtioNet>>>,
    virtio_mmio_allocator: AddressAllocator,
    cmdline_components: Vec<String>,
    input: Box<dyn VMInput>,
    epoll: EpollContext<u32>,
}

pub trait VMInput: std::io::Read + AsRawFd {}
impl<T: std::io::Read + AsRawFd> VMInput for T {}
impl VMM {
    /// Create a new VMM.
    pub fn new(input: Box<dyn VMInput>, output: Box<dyn std::io::Write + Send>) -> Result<Self> {
        // Open /dev/kvm and get a file descriptor to it.
        let kvm = Kvm::new().map_err(Error::KvmIoctl)?;

        // Create a KVM VM object.
        // KVM returns a file descriptor to the VM object.
        let vm_fd = kvm.create_vm().map_err(Error::KvmIoctl)?;

        // Create epoll object
        let epoll: EpollContext<u32> =
            EpollContext::new().map_err(|e| Error::EpollError(e.into()))?;
        epoll
            .add(input.as_ref(), input.as_raw_fd() as u32)
            .map_err(|e| Error::EpollError(e.into()))?;

        const MMIO_GAP_END: u64 = 1 << 32;
        const MMIO_GAP_SIZE: u64 = 768 << 20;
        const MMIO_GAP_START: u64 = MMIO_GAP_END - MMIO_GAP_SIZE;
        let virtio_mmio_allocator =
            AddressAllocator::new(MMIO_GAP_START, 0x2000).map_err(Error::AddressAllocation)?;

        let mut vmm = VMM {
            vm_fd,
            kvm,
            guest_memory: Arc::new(GuestMemoryMmap::default()),
            vcpus: vec![],
            serial: Arc::new(Mutex::new(
                LumperSerial::new(output).map_err(Error::SerialCreation)?,
            )),
            virtio_net: None,
            virtio_mmio_allocator,
            cmdline_components: Vec::new(),
            input,
            epoll,
        };

        vmm.configure_io()?;

        Ok(vmm)
    }

    pub fn configure_memory(&mut self, mem_size_mb: u32) -> Result<()> {
        // Convert memory size from MBytes to bytes.
        let mem_size = ((mem_size_mb as u64) << 20) as usize;

        // Create one single memory region, from zero to mem_size.
        let mem_regions = vec![(GuestAddress(0), mem_size)];
        println!("Memory regions: {:#?}", mem_regions);

        // Allocate the guest memory from the memory region.
        let guest_memory = GuestMemoryMmap::from_ranges(&mem_regions).map_err(Error::Memory)?;

        // For each memory region in guest_memory:
        // 1. Create a KVM memory region mapping the memory region guest physical address to the host virtual address.
        // 2. Register the KVM memory region with KVM. EPTs are created then.
        for (index, region) in guest_memory.iter().enumerate() {
            println!(
                "Registering region start {:x} len {}",
                region.start_addr().raw_value(),
                region.len()
            );
            let kvm_memory_region = kvm_userspace_memory_region {
                slot: index as u32,
                guest_phys_addr: region.start_addr().raw_value(),
                memory_size: region.len() as u64,
                // It's safe to unwrap because the guest address is valid.
                userspace_addr: guest_memory.get_host_address(region.start_addr()).unwrap() as u64,
                flags: 0,
            };

            // Register the KVM memory region with KVM.
            unsafe { self.vm_fd.set_user_memory_region(kvm_memory_region) }
                .map_err(Error::KvmIoctl)?;
        }

        self.guest_memory = Arc::new(guest_memory);

        Ok(())
    }

    pub fn configure_io(&mut self) -> Result<()> {
        // First, create the irqchip.
        // On `x86_64`, this _must_ be created _before_ the vCPUs.
        // It sets up the virtual IOAPIC, virtual PIC, and sets up the future vCPUs for local APIC.
        // When in doubt, look in the kernel for `KVM_CREATE_IRQCHIP`.
        // https://elixir.bootlin.com/linux/latest/source/arch/x86/kvm/x86.c
        self.vm_fd.create_irq_chip().map_err(Error::KvmIoctl)?;

        self.vm_fd
            .register_irqfd(
                &self
                    .serial
                    .lock()
                    .unwrap()
                    .eventfd()
                    .map_err(Error::IrqRegister)?,
                4,
            )
            .map_err(Error::KvmIoctl)?;

        Ok(())
    }

    /// Add a VirtIO network device with TAP backend
    pub fn add_net_device(&mut self, tap_name: Option<&str>) -> Result<()> {
        let mmio_addr = {
            let allocated_range: RangeInclusive = self
                .virtio_mmio_allocator
                .allocate(0x1000, 0x1000, AllocPolicy::FirstMatch)
                .map_err(Error::AddressAllocation)?;
            allocated_range.start()
        };

        let tap = match TapDevice::new(tap_name.unwrap_or("vmtap0")) {
            Ok(t) => {
                println!("Created TAP device: {}", t.name());
                Some(t)
            }
            Err(e) => {
                eprintln!(
                    "Failed to create TAP device: {:?}. Continuing without network.",
                    e
                );
                None
            }
        };

        let mut net = VirtioNet::new(tap, Arc::clone(&self.guest_memory), mmio_addr)
            .map_err(Error::VirtioNetCreation)?;
        net.set_mac([0x42, 0x42, 0x42, 0x42, 0x42, 0x42]);

        let virtio_net = Arc::new(Mutex::new(net));
        self.virtio_net = Some(Arc::clone(&virtio_net));

        self.register_net_mmio(Arc::clone(&virtio_net), mmio_addr)?;
        self.register_net_irq(Arc::clone(&virtio_net), 5)?;
        self.add_net_epoll(Arc::clone(&virtio_net))?;

        self.cmdline_components
            .push(virtio_net.lock().unwrap().cmdline_string(5));

        Ok(())
    }

    fn register_net_mmio(&mut self, net: Arc<Mutex<VirtioNet>>, base_addr: u64) -> Result<()> {
        // Register MMIO memory region with KVM so guest can access VirtIO registers
        // let mmio_addr = self
        //     .guest_memory
        //     .get_host_address(GuestAddress(base_addr))
        //     .expect("Failed to get host address for MMIO");

        // let region = kvm_userspace_memory_region {
        //     slot: 3,
        //     guest_phys_addr: base_addr,
        //     memory_size: 0x1000,
        //     userspace_addr: mmio_addr as u64,
        //     flags: 0,
        // };

        // unsafe {
        //     self.vm_fd
        //         .set_user_memory_region(region)
        //         .map_err(Error::KvmIoctl)?;
        // }

        println!("Registered MMIO region at {:#x} (slot 3)", base_addr);

        // Register ioeventfd for queue notify (offset 0x50)
        let notify_addr = base_addr + 0x50;
        let notify_evt = net
            .lock()
            .unwrap()
            .interrupt_evt()
            .try_clone()
            .expect("Failed to clone interrupt event fd");

        use kvm_ioctls::IoEventAddress;
        let notify_addr_mmio = IoEventAddress::Mmio(notify_addr);
        self.vm_fd
            .register_ioevent(&notify_evt, &notify_addr_mmio, 0u32)
            .map_err(Error::KvmIoctl)?;

        println!("Registered ioeventfd at {:#x}", notify_addr);
        Ok(())
    }

    fn register_net_irq(&mut self, net: Arc<Mutex<VirtioNet>>, irq: u32) -> Result<()> {
        let evt = net
            .lock()
            .unwrap()
            .interrupt_evt()
            .try_clone()
            .map_err(Error::IO)?;

        self.vm_fd
            .register_irqfd(&evt, irq)
            .map_err(Error::KvmIoctl)?;
        Ok(())
    }

    fn add_net_epoll(&mut self, net: Arc<Mutex<VirtioNet>>) -> Result<()> {
        let locked_net = net.lock().unwrap();
        let evt_fd = locked_net.interrupt_evt_fd();
        let evt = locked_net.interrupt_evt();
        self.epoll
            .add(evt, evt_fd as u32)
            .map_err(|e| Error::EpollError(e.into()))?;
        Ok(())
    }

    pub fn configure_vcpus(
        &mut self,
        num_vcpus: u8,
        kernel_load: KernelLoaderResult,
    ) -> Result<()> {
        mptable::setup_mptable(&self.guest_memory, num_vcpus)
            .map_err(|e| Error::Vcpu(cpu::Error::Mptable(e)))?;

        let base_cpuid = self
            .kvm
            .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
            .map_err(Error::KvmIoctl)?;

        for index in 0..num_vcpus {
            let vcpu = Vcpu::new(
                &self.vm_fd,
                index.into(),
                Arc::clone(&self.serial),
                self.virtio_net.clone(),
            )
            .map_err(Error::Vcpu)?;

            // Set CPUID.
            let mut vcpu_cpuid = base_cpuid.clone();
            cpuid::filter_cpuid(
                &self.kvm,
                index as usize,
                num_vcpus as usize,
                &mut vcpu_cpuid,
            );
            vcpu.configure_cpuid(&vcpu_cpuid).map_err(Error::Vcpu)?;

            // Configure MSRs (model specific registers).
            vcpu.configure_msrs().map_err(Error::Vcpu)?;

            // Configure regs, sregs and fpu.
            vcpu.configure_regs(kernel_load.kernel_load)
                .map_err(Error::Vcpu)?;
            vcpu.configure_sregs(&self.guest_memory)
                .map_err(Error::Vcpu)?;
            vcpu.configure_fpu().map_err(Error::Vcpu)?;

            // Configure LAPICs.
            vcpu.configure_lapic().map_err(Error::Vcpu)?;

            self.vcpus.push(vcpu);
        }

        Ok(())
    }

    // Run all virtual CPUs.
    pub fn run(&mut self) {
        for mut vcpu in self.vcpus.drain(..) {
            println!("Starting vCPU {:?}", vcpu.index);
            let _ = thread::Builder::new().spawn(move || loop {
                vcpu.run();
            });
        }

        self.host_epoll_blocking()
            .expect("epoll loop should live forever") // TODO handle stdin failures gracefully
    }

    // Blocking function to poll various fd from host using epoll (e.g. stdin)
    pub fn host_epoll_blocking(&mut self) -> Result<()> {
        // poll epoll fd using infinite loop
        let events = EpollEvents::new();
        loop {
            for event in self.epoll.wait(&events).unwrap().iter_readable() {
                if event.token() == self.input.as_raw_fd() as u32 {
                    // input token
                    let mut out = [0u8; 64];

                    let count = self.input.read(&mut out).map_err(Error::StdinRead)?;

                    self.serial
                        .lock()
                        .unwrap()
                        .serial
                        .enqueue_raw_bytes(&out[..count])
                        .map_err(Error::StdinWrite)?;
                }

                // Handle VirtIO network device events
                if let Some(ref virtio_net) = self.virtio_net {
                    let net = virtio_net.lock().unwrap();

                    // Check if this is an interrupt event
                    if event.token() == net.interrupt_evt().as_raw_fd() as u32 {
                        // Acknowledge interrupt
                        net.interrupt_status()
                            .fetch_and(!0x1, std::sync::atomic::Ordering::SeqCst);
                    }
                }
            }
        }
    }

    pub fn configure(
        &mut self,
        num_vcpus: u8,
        mem_size_mb: u32,
        kernel_path: &str,
        initramfs_path: &str,
    ) -> Result<()> {
        self.configure_memory(mem_size_mb)?;
        let kernel_load = kernel::kernel_setup(
            &self.guest_memory,
            PathBuf::from(kernel_path),
            Some(PathBuf::from(initramfs_path)),
            self.cmdline_components.clone(),
        )?;
        self.configure_vcpus(num_vcpus, kernel_load)?;

        Ok(())
    }
}
