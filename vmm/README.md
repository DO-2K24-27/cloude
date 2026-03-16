# cloude-vmm

The `cloude-vmm` is a Virtual Machine Manager (VMM) designed to manage the lifecycle of micro-VMs. It intercepts and virtualizes CPU, memory, and I/O access to run multiple isolated virtual machines on the same physical system, while providing low-level abstractions over KVM (Kernel-based Virtual Machine) to handle the creation, configuration, and lifecycle management of micro-VMs.

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

## Key Functions

Below are the most important functions implemented in the `cloude-vmm` and their roles:

### 1. `create_vm`
- **Purpose**: Initializes and configures a new virtual machine.
- **Parameters**:
  - `kernel_path: &Path`: Path to the kernel binary to load into the VM.
  - `memory_size: usize`: Amount of memory (in bytes) to allocate for the VM.
  - `num_cpus: u8`: Number of virtual CPUs to assign to the VM.
- **Returns**:
  - `Result<Vm, VmError>`: The created VM instance on success, or an error on failure.
- **Details**:
  - Allocates memory for the guest VM.
  - Loads the kernel and configures the initial CPU state.
  - Sets up virtual devices such as block storage and network interfaces.

### 2. `run_vm`
- **Purpose**: Starts the execution of the virtual machine.
- **Parameters**:
  - `vm: &mut Vm`: The virtual machine instance to run.
- **Returns**:
  - `Result<(), VmExitError>`: Indicates whether the VM ran successfully or encountered an exit error.
- **Details**:
  - Enters the KVM run loop to execute the VM.
  - Handles VM exits by inspecting the `VcpuExit` enum.
  - Processes specific exit reasons such as I/O operations, MMIO, and shutdown signals.

### 3. `load_kernel`
- **Purpose**: Loads the guest kernel into the VM's memory.
- **Parameters**:
  - `vm: &mut Vm`: The virtual machine instance.
  - `kernel_path: &Path`: Path to the kernel binary to load.
- **Returns**:
  - `Result<(), KernelLoadError>`: Indicates whether the kernel was successfully loaded.
- **Details**:
  - Parses the kernel ELF file to determine the entry point and memory layout.
  - Copies the kernel binary into the allocated memory region.
  - Configures the initial CPU state to start execution at the kernel's entry point.

### 4. `setup_virtio_devices`
- **Purpose**: Configures Virtio devices for the guest VM.
- **Parameters**:
  - `vm: &mut Vm`: The virtual machine instance.
  - `devices: Vec<VirtioDevice>`: A list of Virtio devices to attach to the VM.
- **Returns**:
  - `Result<(), DeviceSetupError>`: Indicates whether the devices were successfully configured.
- **Details**:
  - Initializes Virtio block and network devices.
  - Allocates memory regions and IRQs for each device.
  - Sets up communication channels between the guest and the host.

### 5. `handle_vm_exit`
- **Purpose**: Processes VM exits during execution.
- **Parameters**:
  - `exit_reason: VcpuExit`: The reason for the VM exit.
- **Returns**:
  - `Result<(), VmExitError>`: Indicates whether the exit was handled successfully or encountered an error.
- **Details**:
  - Matches the `VcpuExit` enum to determine the exit reason.
  - Handles specific exits such as:
    - `VcpuExit::IoOut` and `VcpuExit::IoIn` for I/O port operations.
    - `VcpuExit::MmioRead` and `VcpuExit::MmioWrite` for memory-mapped I/O.
    - `VcpuExit::Shutdown` and `VcpuExit::Hlt` for guest shutdown.
  - Logs and handles unhandled exits gracefully.

### 6. `allocate_irq`
- **Purpose**: Allocates IRQ lines for virtual devices.
- **Parameters**:
  - `irq_allocator: &mut IrqAllocator`: The IRQ allocator instance.
- **Returns**:
  - `Result<u32, IrqAllocationError>`: The allocated IRQ line on success, or an error on failure.
- **Details**:
  - Ensures that each device is assigned a unique IRQ line.
  - Tracks allocated IRQs to prevent conflicts.

### 7. `setup_network`
- **Purpose**: Configures networking for the guest VM.
- **Parameters**:
  - `vm: &mut Vm`: The virtual machine instance.
  - `tap_device: &TapDevice`: The TAP device to use for networking.
- **Returns**:
  - `Result<(), NetworkSetupError>`: Indicates whether the network was successfully configured.
- **Details**:
  - Sets up a TAP device on the host to bridge the guest's network interface.
  - Configures NAT to enable internet access for the guest.
  - Assigns a unique IP address to the VM.

### 8. `shutdown_vm`
- **Purpose**: Gracefully shuts down the virtual machine.
- **Parameters**:
  - `vm: &mut Vm`: The virtual machine instance to shut down.
- **Returns**:
  - `Result<(), VmShutdownError>`: Indicates whether the VM was successfully shut down.
- **Details**:
  - Signals all vCPUs to exit.
  - Cleans up allocated resources such as memory and devices.
  - Ensures that the VM is properly terminated to avoid resource leaks.

## Code Structure

- `src/kernel.rs`: Kernel loading and configuration.
- `src/irq_allocator.rs`: IRQ allocation logic.
- `src/devices/`: Virtual device implementations, including Virtio network.
- `src/network.rs`: Networking setup for VMs, including TAP device configuration and Virtio network integration.
- `src/cpu/`: vCPU setup and management.

## What it provides

- Load kernel + initramfs
- Configure vCPUs and guest memory
- Optional VirtIO net (tap-backed)
- Event loop + serial handling
- Graceful stop support from another thread

## Main API

- `VMM::new(input, output, memory_size)`
- `add_net_device(tap, guest_ip, host_ip, netmask)`
- `configure(vcpus, kernel_path, initramfs_path, init_path)`
- `run()`
- `stop()`
- `stop_handle()`

`stop_handle()` exposes the internal running flag used by `run()` and vCPU
threads. Setting it to `false` requests a graceful shutdown.

You can look at `backend/virt/src/bin/run_vm.rs` for an example
