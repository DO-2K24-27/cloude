# VMM Documentation

## Overview

The `vmm` is a Virtual Machine Manager (VMM) designed to manage the lifecycle of micro-VMs. It intercepts and virtualizes CPU, memory, and I/O access to run multiple isolated virtual machines on the same physical system, while providing low-level abstractions over KVM (Kernel-based Virtual Machine) to handle the creation, configuration, and lifecycle management of micro-VMs.

## Implemented Features

### 1. IRQ Allocator
- **Purpose**: Manages the allocation of interrupt request (IRQ) lines for virtual devices.
- **Details**:
  - Ensures that each virtual device is assigned a unique IRQ line.
  - Tracks allocated IRQs to prevent conflicts.
  - Provides methods to allocate and free IRQs dynamically.

### 2. Kernel Loader
- **Purpose**: Loads the kernel binary into the guest VM's memory.
- **Details**:
  - Parses the kernel ELF file to extract the entry point and memory layout.
  - Copies the kernel image into the guest's memory space.
  - Configures the initial CPU state to start execution at the kernel's entry point.

### 3. Virtual Devices
- **Purpose**: Manages the creation and configuration of virtual devices for the guest VM.
- **Implemented Devices**:
  - **Virtio Block Device**: Provides block storage to the guest.
  - **Virtio Network Device**: Enables network communication for the guest.
  - **Serial Console**: Captures the guest's console output.
- **Details**:
  - Configures device memory regions and IRQs.
  - Handles communication between the guest and the host for each device.

### 4. CPU Configuration
- **Purpose**: Sets up the virtual CPUs (vCPUs) for the guest VM.
- **Details**:
  - Configures the initial state of each vCPU, including registers and control flags.
  - Supports multi-core configurations.
  - Integrates with KVM to manage vCPU execution.

### 5. Memory Management
- **Purpose**: Allocates and maps memory for the guest VM.
- **Details**:
  - Allocates guest physical memory using `mmap`.
  - Maps memory regions for the kernel, initramfs, and virtual devices.
  - Ensures proper alignment and permissions for memory regions.

### 6. Networking
- **Purpose**: Provides network connectivity to the guest VM.
- **Details**:
  - Sets up a TAP (network tap) device for the guest.
  - Configures NAT (Network Address Translation) to enable internet access.
  - Assigns unique IP addresses to each VM.

### 7. VM Lifecycle Management
- **Purpose**: Handles the creation, execution, and termination of VMs.
- **Details**:
  - Provides APIs to start, stop, and reset VMs.
  - Monitors VM state and resource usage.
  - Cleans up resources when a VM is terminated.

### 8. Virtio Network Integration
- **Purpose**: Implements the Virtio network device to provide efficient and standardized network communication for the guest VM.
- **Details**:
  - **Device Initialization**: Sets up the Virtio network device during VM creation, including memory mapping and feature negotiation.
  - **Packet Transmission**: Handles the transmission of network packets between the guest and the host using the Virtio queue mechanism.
  - **Packet Reception**: Processes incoming packets from the host and delivers them to the guest via the Virtio queue.
  - **TAP Device**: Utilizes a TAP (network tap) device on the host to bridge the guest's network interface with the host's network stack.
  - **Performance Optimization**: Implements features like checksum offloading and scatter-gather I/O to improve network performance.