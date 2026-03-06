# vmm

Custom virtual machine monitor (VMM) for KVM-based virtualization.

## Overview

The vmm component is a lightweight virtual machine monitor that provides direct KVM integration for running Linux guests. It implements core virtualization features including memory management, vCPU execution, device emulation, and interrupt handling.

## Features

- **KVM Integration**: Direct interaction with Linux KVM via ioctl system calls
- **Memory Management**: Guest physical memory allocation and mapping
- **vCPU Management**: Configuration and execution of virtual CPUs
- **Device Emulation**:
  - Serial console (UART 16550)
  - VirtIO network device (virtio-net)
  - Standard PC platform devices
- **Interrupt Handling**: IRQ allocation and management
- **Linux Boot**: Kernel loading and boot parameter configuration

## Architecture

The VMM is structured around a core `VMM` struct that owns:
- KVM file descriptor for hypervisor access
- VM file descriptor for VM-level operations
- Guest memory regions
- vCPU instances
- Device emulators
- Event management for I/O

### Key Components

#### cpu/
- **cpuid.rs**: CPUID instruction emulation for CPU feature detection
- **mptable.rs**: MP (multiprocessor) table generation for SMP support
- **vcpu.rs**: Virtual CPU lifecycle and execution

#### devices/
- **serial.rs**: Serial console (LumperSerial) for VM I/O
- **stdin.rs**: Host stdin forwarding to guest
- **virtio/net**: VirtIO network device implementation

#### Memory Layout

The VMM configures guest memory with:
- Low memory: 0 to MMIO_GAP_START
- MMIO gap: Reserved for memory-mapped I/O devices
- High memory: Above MMIO_GAP_END (if configured)

Default MMIO gap: 768 MB ending at 4 GB boundary

## Usage

The VMM is primarily used as a library. The test binary demonstrates basic usage:

```bash
# Set required environment variables
export KERNEL_PATH="/path/to/vmlinuz"
export INITRAMFS_PATH="/path/to/initramfs.cpio.gz"

# Optional: Enable serial output logging
export SERIAL_OUTPUT="/tmp/vm-serial.log"

# Optional: Enable networking
export TAP_DEVICE="tap0"
export GUEST_IP="192.168.100.2"
export HOST_IP="192.168.100.1"
export NETMASK="255.255.255.0"

# Run test VMM
cargo run -p vmm --bin test
```

## Requirements

- **Platform**: Linux x86_64 only
- **KVM**: `/dev/kvm` must be accessible
- **Permissions**: User must have access to KVM device
- **Dependencies**: 
  - kvm-ioctls
  - vm-memory
  - vm-superio
  - linux-loader
  - event-manager

## Configuration

### vCPU Count
Configured programmatically when creating VMM instance. Default: 1 vCPU

### Memory Size
Specified as guest memory size in bytes. Example: 512 MB

### Network
Network support requires:
1. Pre-configured TAP device on host
2. Environment variables for IP configuration
3. VirtIO-net device enabled

## API

### Creating a VMM

```rust
use vmm::VMM;

// Create VMM with 1 vCPU and 512 MB RAM
let mut vmm = VMM::new(1, 512 << 20)?;

// Configure kernel and initramfs
vmm.configure_console()?;
vmm.load_kernel(kernel_path)?;
vmm.load_initramfs(initramfs_path)?;

// Run VM
vmm.run()?;
```

### Key Methods

- `new(num_vcpus, mem_size)`: Initialize VMM instance
- `configure_console()`: Set up serial console device
- `load_kernel(path)`: Load Linux kernel into guest memory
- `load_initramfs(path)`: Load initramfs into guest memory
- `run()`: Start vCPU execution loop
- `add_virtio_net(tap_name, guest_ip, host_ip, netmask)`: Configure network

## Error Handling

The VMM uses a custom `Error` enum covering:
- KVM ioctl errors
- Memory configuration errors
- Kernel loading failures
- Device creation errors
- I/O errors

## Limitations

- **x86_64 only**: No support for other architectures
- **Linux host**: Requires Linux KVM
- **Single VM**: One VM per VMM instance
- **Basic devices**: Limited device emulation (serial, virtio-net)

## Development

### Building

```bash
cargo build -p vmm
```

### Testing

```bash
# Run with debug logging
export RUST_LOG=debug
cargo run -p vmm --bin test
```

## License

See [LICENSE](LICENSE) file for details.

## References

- [KVM API Documentation](https://www.kernel.org/doc/Documentation/virtual/kvm/api.txt)
- [VirtIO Specification](https://docs.oasis-open.org/virtio/virtio/v1.1/virtio-v1.1.html)
- [Linux Boot Protocol](https://www.kernel.org/doc/Documentation/x86/boot.txt)
