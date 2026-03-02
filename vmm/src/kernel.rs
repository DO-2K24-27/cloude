// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

#![cfg(target_arch = "x86_64")]

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::result;

use linux_loader::bootparam::boot_params;
use linux_loader::cmdline::Cmdline;
use linux_loader::configurator::{linux::LinuxBootConfigurator, BootConfigurator, BootParams};
use linux_loader::loader::{elf::Elf, load_cmdline, KernelLoader, KernelLoaderResult};
use vm_memory::{Address, GuestAddress, GuestMemory, GuestMemoryMmap, GuestMemoryRegion};

use crate::{Error, Result};

// x86_64 boot constants. See https://www.kernel.org/doc/Documentation/x86/boot.txt for the full
// documentation.
// Header field: `boot_flag`. Must contain 0xaa55. This is the closest thing old Linux kernels
// have to a magic number.
const KERNEL_BOOT_FLAG_MAGIC: u16 = 0xaa55;
// Header field: `header`. Must contain the magic number `HdrS` (0x5372_6448).
const KERNEL_HDR_MAGIC: u32 = 0x5372_6448;
// Header field: `type_of_loader`. Unless using a pre-registered bootloader (which we aren't), this
// field must be set to 0xff.
const KERNEL_LOADER_OTHER: u8 = 0xff;
// Header field: `kernel_alignment`. Alignment unit required by a relocatable kernel.
const KERNEL_MIN_ALIGNMENT_BYTES: u32 = 0x0100_0000;

// Start address for the EBDA (Extended Bios Data Area). Older computers (like the one this VMM
// emulates) typically use 1 KiB for the EBDA, starting at 0x9fc00.
// See https://wiki.osdev.org/Memory_Map_(x86) for more information.
const EBDA_START: u64 = 0x0009_fc00;
// RAM memory type.
// TODO: this should be bindgen'ed and exported by linux-loader.
// See https://github.com/rust-vmm/linux-loader/issues/51
const E820_RAM: u32 = 1;

/// Address of the zeropage, where Linux kernel boot parameters are written.
pub(crate) const ZEROPG_START: u64 = 0x7000;

const HIMEM_START: u64 = 0x0010_0000; // 1 MB

/// Address where the initramfs is loaded (128 MB, well after kernel)
const INITRAMFS_START: u64 = 0x0800_0000; // 128 MB

/// Address where the kernel command line is written.
const CMDLINE_START: u64 = 0x0002_0000;
/// Maximum size for kernel command line
const CMDLINE_MAX_SIZE: usize = 4096;
// Default command line
const CMDLINE: &str = "console=ttyS0 i8042.nokbd reboot=t panic=1 pci=off";

fn add_e820_entry(
    params: &mut boot_params,
    addr: u64,
    size: u64,
    mem_type: u32,
) -> result::Result<(), Error> {
    if params.e820_entries >= params.e820_table.len() as u8 {
        return Err(Error::E820Configuration);
    }

    params.e820_table[params.e820_entries as usize].addr = addr;
    params.e820_table[params.e820_entries as usize].size = size;
    params.e820_table[params.e820_entries as usize].type_ = mem_type;
    params.e820_entries += 1;

    Ok(())
}

/// Build boot parameters for ELF kernels following the Linux boot protocol.
///
/// # Arguments
///
/// * `guest_memory` - guest memory
/// * `himem_start` - address where high memory starts.
/// * `mmio_gap_start` - address where the MMIO gap starts.
/// * `mmio_gap_end` - address where the MMIO gap ends.
pub fn build_bootparams(
    guest_memory: &GuestMemoryMmap,
    himem_start: GuestAddress,
) -> std::result::Result<boot_params, Error> {
    let mut params = boot_params::default();

    params.hdr.boot_flag = KERNEL_BOOT_FLAG_MAGIC;
    params.hdr.header = KERNEL_HDR_MAGIC;
    params.hdr.kernel_alignment = KERNEL_MIN_ALIGNMENT_BYTES;
    params.hdr.type_of_loader = KERNEL_LOADER_OTHER;

    // Add an entry for EBDA itself.
    add_e820_entry(&mut params, 0, EBDA_START, E820_RAM)?;

    // Add entries for the usable RAM regions.
    let last_addr = guest_memory.last_addr();
    add_e820_entry(
        &mut params,
        himem_start.raw_value() as u64,
        last_addr
            .checked_offset_from(himem_start)
            .ok_or(Error::HimemStartPastMemEnd)?,
        E820_RAM,
    )?;

    Ok(params)
}

/// Set guest kernel up.
///
/// # Arguments
///
/// * `guest_memory` - Guest memory
/// * `kernel_path` - Path to the kernel image
/// * `initramfs_path` - Optional path to the initramfs image
pub fn kernel_setup(
    guest_memory: &GuestMemoryMmap,
    kernel_path: PathBuf,
    initramfs_path: Option<PathBuf>,
    cmdline_components: Vec<String>,
) -> Result<KernelLoaderResult> {
    let mut kernel_image = File::open(kernel_path).map_err(Error::IO)?;
    let zero_page_addr = GuestAddress(ZEROPG_START);

    // Load the kernel into guest memory.
    let kernel_load = Elf::load(
        guest_memory,
        None,
        &mut kernel_image,
        Some(GuestAddress(HIMEM_START)),
    )
    .map_err(Error::KernelLoad)?;

    // Generate boot parameters.
    let mut bootparams = build_bootparams(guest_memory, GuestAddress(HIMEM_START))?;

    // Build the kernel command line
    let mut cmdline = Cmdline::new(CMDLINE_MAX_SIZE);
    cmdline.insert_str(CMDLINE).map_err(Error::Cmdline)?;
    for cmdline_str in cmdline_components {
        cmdline.insert_str(&cmdline_str).map_err(Error::Cmdline)?;
        cmdline.insert_str(" ").map_err(Error::Cmdline)?;
    }

    // Load initramfs if provided
    if let Some(initramfs_path) = initramfs_path {
        let (initramfs_addr, initramfs_size) = load_initramfs(guest_memory, initramfs_path)?;

        // Add initramfs location to boot parameters
        bootparams.hdr.ramdisk_image = initramfs_addr.raw_value() as u32;
        bootparams.hdr.ramdisk_size = initramfs_size as u32;

        // Add rdinit to command line
        cmdline
            .insert_str(" rdinit=/bin/sh")
            .map_err(Error::Cmdline)?;

        println!(
            "Initramfs loaded: {} bytes at 0x{:x}",
            initramfs_size,
            initramfs_addr.raw_value()
        );
    }

    // Add the kernel command line to the boot parameters.
    bootparams.hdr.cmd_line_ptr = CMDLINE_START as u32;
    bootparams.hdr.cmdline_size = cmdline.as_str().len() as u32 + 1;

    // Load the kernel command line into guest memory.
    load_cmdline(guest_memory, GuestAddress(CMDLINE_START), &cmdline).map_err(Error::KernelLoad)?;

    // Write the boot parameters in the zeropage.
    LinuxBootConfigurator::write_bootparams::<GuestMemoryMmap>(
        &BootParams::new::<boot_params>(&bootparams, zero_page_addr),
        guest_memory,
    )
    .map_err(Error::BootConfigure)?;

    Ok(kernel_load)
}

/// Load initramfs into guest memory.
///
/// # Arguments
///
/// * `guest_memory` - Guest memory
/// * `initramfs_path` - Path to the initramfs image
///
/// # Returns
///
/// Tuple of (address, size) where the initramfs was loaded
fn load_initramfs(
    guest_memory: &GuestMemoryMmap,
    initramfs_path: PathBuf,
) -> Result<(GuestAddress, usize)> {
    let mut initramfs_file = File::open(&initramfs_path).map_err(Error::IO)?;

    // Read the entire initramfs into memory
    let mut initramfs_data = Vec::new();
    initramfs_file
        .read_to_end(&mut initramfs_data)
        .map_err(Error::IO)?;

    let initramfs_size = initramfs_data.len();
    let initramfs_addr = GuestAddress(INITRAMFS_START);

    // Verify we have enough guest memory
    let required_addr = initramfs_addr
        .checked_add(initramfs_size as u64)
        .ok_or(Error::HimemStartPastMemEnd)?;

    if required_addr > guest_memory.last_addr() {
        return Err(Error::HimemStartPastMemEnd);
    }

    // Write initramfs to guest memory region by region
    // Find the region that contains our initramfs address
    for region in guest_memory.iter() {
        if initramfs_addr >= region.start_addr() && initramfs_addr <= region.last_addr() {
            // This region contains the start of our initramfs
            let offset_in_region =
                (initramfs_addr.raw_value() - region.start_addr().raw_value()) as usize;

            // Get the volatile slice
            let vol_slice = region
                .as_volatile_slice()
                .map_err(|_| Error::HimemStartPastMemEnd)?;

            // Get the subslice at the right offset
            let slice = vol_slice
                .subslice(offset_in_region, initramfs_size)
                .map_err(|_| Error::HimemStartPastMemEnd)?;

            // Copy the data to guest memory
            slice.copy_from(&initramfs_data[..]);

            break;
        }
    }

    Ok((initramfs_addr, initramfs_size))
}
