// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

#![cfg(target_arch = "x86_64")]

extern crate libc;

extern crate linux_loader;
extern crate vm_memory;
extern crate vm_superio;

use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::{io, path::PathBuf};

use event_manager::{EventManager, MutEventSubscriber, SubscriberOps};
use kvm_bindings::{kvm_userspace_memory_region, KVM_MAX_CPUID_ENTRIES};
use kvm_ioctls::{Kvm, VmFd};
use linux_loader::loader::{self, KernelLoaderResult};
use vm_allocator::{AddressAllocator, AllocPolicy, RangeInclusive};
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};
mod cpu;
use cpu::{cpuid, mptable, Vcpu};
mod devices;
use devices::serial::LumperSerial;
use devices::stdin::StdinHandler;

use crate::devices::virtio::net::device::VirtioNetDevice;
use crate::irq_allocator::IrqAllocator;

mod irq_allocator;
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
    Virtio(devices::virtio::Error),
}

/// Dedicated [`Result`](https://doc.rust-lang.org/std/result/) type.
pub type Result<T> = std::result::Result<T, Error>;

pub struct VMM {
    vm_fd: Arc<VmFd>,
    kvm: Kvm,
    guest_memory: Arc<GuestMemoryMmap>,
    vcpus: Vec<Vcpu>,
    serial: Arc<Mutex<LumperSerial>>,
    virtio_net: Option<Arc<Mutex<VirtioNetDevice>>>,
    cmdline_components: Vec<String>,
    event_manager: EventManager<Arc<Mutex<dyn MutEventSubscriber>>>,
    virtio_mmio_allocator: AddressAllocator,
    irq_allocator: IrqAllocator,
    running: Arc<AtomicBool>,
    vcpu_handles: Vec<thread::JoinHandle<()>>,
    vcpu_thread_ids: Arc<Mutex<Vec<libc::pthread_t>>>,
}

pub trait VMInput: std::io::Read + AsRawFd {}
impl<T: std::io::Read + AsRawFd> VMInput for T {}
impl VMM {
    /// Create a new VMM.
    pub fn new(
        input: Box<dyn VMInput>,
        output: Box<dyn std::io::Write + Send>,
        memory_size: usize,
    ) -> Result<Self> {
        // Create a KVM VM object.
        let kvm = Kvm::new().map_err(Error::KvmIoctl)?;
        let vm_fd = kvm.create_vm().map_err(Error::KvmIoctl)?;

        // Create event manager
        let mut event_manager: EventManager<Arc<Mutex<dyn MutEventSubscriber>>> =
            EventManager::new().map_err(|e| {
                Error::EpollError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;

        let virtio_mmio_allocator =
            AddressAllocator::new(MMIO_GAP_START, 0x2000).map_err(Error::AddressAllocation)?;

        let guest_memory = Self::configure_memory(&vm_fd, memory_size)?;

        let serial = Arc::new(Mutex::new(
            LumperSerial::new(output).map_err(Error::SerialCreation)?,
        ));

        // Create stdin handler and add it to event manager
        let stdin_handler: Arc<Mutex<dyn MutEventSubscriber>> =
            Arc::new(Mutex::new(StdinHandler::new(input, serial.clone())));
        event_manager.add_subscriber(stdin_handler);

        let mut vmm = VMM {
            vm_fd: Arc::new(vm_fd),
            kvm,
            guest_memory: Arc::new(guest_memory),
            vcpus: vec![],
            serial,
            virtio_net: None,
            virtio_mmio_allocator,
            cmdline_components: Vec::new(),
            event_manager,
            irq_allocator: IrqAllocator::new(5),
            running: Arc::new(AtomicBool::new(true)),
            vcpu_handles: Vec::new(),
            vcpu_thread_ids: Arc::new(Mutex::new(Vec::new())),
        };

        vmm.configure_io()?;

        Ok(vmm)
    }

    fn configure_memory(vm_fd: &VmFd, memory_size: usize) -> Result<GuestMemoryMmap> {
        let guest_memory = GuestMemoryMmap::from_ranges(&[(GuestAddress(0), memory_size)])
            .map_err(Error::Memory)?;

        for (index, region) in guest_memory.iter().enumerate() {
            let kvm_memory_region = kvm_userspace_memory_region {
                slot: index as u32,
                guest_phys_addr: region.start_addr().raw_value(),
                memory_size: region.len() as u64,
                // It's safe to unwrap because the guest address is valid.
                userspace_addr: guest_memory.get_host_address(region.start_addr()).unwrap() as u64,
                flags: 0,
            };

            // Register the KVM memory region with KVM.
            unsafe { vm_fd.set_user_memory_region(kvm_memory_region) }.map_err(Error::KvmIoctl)?;
        }

        Ok(guest_memory)
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
    pub fn add_net_device(&mut self, tap_name: String) -> Result<()> {
        let allocated_range: RangeInclusive = self
            .virtio_mmio_allocator
            .allocate(0x1000, 0x1000, AllocPolicy::FirstMatch)
            .map_err(Error::AddressAllocation)?;

        let irq = self.irq_allocator.allocate();

        let endpoint = self.event_manager.remote_endpoint();

        let net = VirtioNetDevice::new(
            self.vm_fd.clone(),
            irq,
            tap_name,
            self.guest_memory.clone(),
            allocated_range,
            endpoint,
        )
        .map_err(Error::Virtio)?;

        self.cmdline_components.push(net.cmdline_string());

        let virtio_net = Arc::new(Mutex::new(net));
        self.virtio_net = Some(Arc::clone(&virtio_net));

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
                Arc::clone(&self.running),
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

    fn start_vcpus(&mut self) {
        for mut vcpu in self.vcpus.drain(..) {
            println!("Starting vCPU {:?}", vcpu.index);
            let vcpu_running = Arc::clone(&self.running);
            let thread_ids = Arc::clone(&self.vcpu_thread_ids);
            let handle = thread::Builder::new()
                .spawn(move || {
                    thread_ids
                        .lock()
                        .unwrap()
                        .push(unsafe { libc::pthread_self() });

                    while vcpu_running.load(Ordering::SeqCst) {
                        vcpu.run();
                    }
                })
                .expect("Failed to spawn vCPU thread");
            self.vcpu_handles.push(handle);
        }
    }

    /// Wait for all vCPU threads to finish, sending SIGUSR1 to interrupt
    /// any threads blocked in KVM_RUN.
    fn join_vcpus(&mut self) {
        let tids = self.vcpu_thread_ids.lock().unwrap();
        for &tid in tids.iter() {
            unsafe {
                libc::pthread_kill(tid, libc::SIGUSR1);
            }
        }
        drop(tids);

        for handle in self.vcpu_handles.drain(..) {
            let _ = handle.join();
        }
        self.vcpu_thread_ids.lock().unwrap().clear();
    }

    /// Run the VM: start vCPUs, run event loop, and wait for shutdown.
    pub fn run(&mut self) {
        self.running.store(true, Ordering::SeqCst);

        // Install a no-op SIGUSR1 handler so pthread_kill interrupts KVM_RUN
        // with EINTR instead of terminating the process.
        unsafe {
            let mut sa: libc::sigaction = std::mem::zeroed();
            sa.sa_sigaction = empty_signal_handler as usize;
            sa.sa_flags = 0;
            libc::sigaction(libc::SIGUSR1, &sa, std::ptr::null_mut());
        }

        self.start_vcpus();

        let running = Arc::clone(&self.running);
        while running.load(Ordering::SeqCst) {
            self.event_manager
                .run_with_timeout(100)
                .expect("event manager loop should live forever");
        }

        self.join_vcpus();
    }

    /// Stop the VM by signaling all threads to exit.
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn configure(
        &mut self,
        num_vcpus: u8,
        kernel_path: &str,
        initramfs_path: &str,
    ) -> Result<()> {
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

/// No-op signal handler used to interrupt vCPU threads blocked in KVM_RUN.
extern "C" fn empty_signal_handler(_: libc::c_int) {}
