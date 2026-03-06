# Environment Variables

This document lists all environment variables used across the cloude platform components.

## agent

### AGENT_SERVER_ADDR
- **Description**: Address and port for the agent HTTP server to bind to
- **Type**: String (format: "host:port")
- **Default**: `127.0.0.1:3001`
- **Required**: No
- **Example**: `0.0.0.0:3001`

### AGENT_WORK_DIR
- **Description**: Directory for storing temporary build artifacts (initramfs images)
- **Type**: Path (relative or absolute)
- **Default**: `build`
- **Required**: No
- **Example**: `/tmp/agent-builds`
- **Notes**: Directory is created automatically if it doesn't exist

### AGENT_KERNEL_PATH
- **Description**: Path to the Linux kernel image (vmlinuz) used for QEMU VMs
- **Type**: Path (absolute)
- **Default**: None
- **Required**: Yes
- **Example**: `$(pwd)/agent/.cache/kernels/vmlinuz-virt-3.23.3`
- **Notes**: 
  - Kernel must be a bootable Linux kernel image
  - Alpine Linux kernels are recommended
  - Use `agent/scripts/fetch-kernel.sh` to download a compatible kernel

### AGENT_QEMU_TIMEOUT_SECS
- **Description**: Maximum execution time for QEMU VM processes in seconds
- **Type**: Integer (unsigned 64-bit)
- **Default**: `120`
- **Required**: No
- **Example**: `300`
- **Notes**: VMs exceeding this timeout will be killed, preventing runaway processes

## backend

### BACKEND_SERVER_ADDR
- **Description**: Address and port for the backend HTTP server to bind to
- **Type**: String (format: "host:port")
- **Default**: `127.0.0.1:8080`
- **Required**: No
- **Example**: `0.0.0.0:8080`

## vmm

### KERNEL_PATH
- **Description**: Path to the kernel image for VMM execution
- **Type**: Path (absolute)
- **Default**: None
- **Required**: Yes
- **Example**: `/path/to/vmlinuz`
- **Used in**: `vmm/src/bin/test.rs`

### INITRAMFS_PATH
- **Description**: Path to the initramfs image for VMM execution
- **Type**: Path (absolute)
- **Default**: None
- **Required**: Yes
- **Example**: `/path/to/initramfs.cpio.gz`
- **Used in**: `vmm/src/bin/test.rs`

### SERIAL_OUTPUT
- **Description**: Path to write serial console output from the VM
- **Type**: Path (absolute)
- **Default**: None (output to stdout)
- **Required**: No
- **Example**: `/tmp/vm-serial.log`
- **Used in**: `vmm/src/bin/test.rs`

### TAP_DEVICE
- **Description**: Name of the TAP network device for VM networking
- **Type**: String
- **Default**: None (no networking)
- **Required**: No
- **Example**: `tap0`
- **Notes**: Requires network configuration variables below

### GUEST_IP
- **Description**: IP address to assign to the guest VM
- **Type**: IPv4 address
- **Default**: None
- **Required**: Only if TAP_DEVICE is set
- **Example**: `192.168.100.2`

### HOST_IP
- **Description**: IP address of the host on the TAP network
- **Type**: IPv4 address
- **Default**: None
- **Required**: Only if TAP_DEVICE is set
- **Example**: `192.168.100.1`

### NETMASK
- **Description**: Network mask for the TAP network
- **Type**: IPv4 netmask
- **Default**: None
- **Required**: Only if TAP_DEVICE is set
- **Example**: `255.255.255.0`

### RUST_LOG
- **Description**: Logging level for Rust tracing
- **Type**: String (trace, debug, info, warn, error)
- **Default**: None
- **Required**: No
- **Example**: `debug`
- **Notes**: Standard Rust logging environment variable

### VERBOSE
- **Description**: Enable verbose logging output
- **Type**: Any value (presence is checked)
- **Default**: None
- **Required**: No
- **Example**: `1`

## cli

The CLI component does not use environment variables. Configuration is provided via command-line arguments.

## Configuration Examples

### Development Setup

```bash
# Agent
export AGENT_SERVER_ADDR="127.0.0.1:3001"
export AGENT_WORK_DIR="build"
export AGENT_KERNEL_PATH="$(pwd)/agent/.cache/kernels/vmlinuz-virt-3.23.3"
export AGENT_QEMU_TIMEOUT_SECS="120"

# Backend
export BACKEND_SERVER_ADDR="127.0.0.1:8080"
```

### Production Setup

```bash
# Agent
export AGENT_SERVER_ADDR="0.0.0.0:3001"
export AGENT_WORK_DIR="/var/lib/cloude/builds"
export AGENT_KERNEL_PATH="/opt/cloude/kernels/vmlinuz-virt-3.23.3"
export AGENT_QEMU_TIMEOUT_SECS="300"

# Backend
export BACKEND_SERVER_ADDR="0.0.0.0:8080"
```

### VMM Testing

```bash
export KERNEL_PATH="/path/to/vmlinuz"
export INITRAMFS_PATH="/path/to/initramfs.cpio.gz"
export SERIAL_OUTPUT="/tmp/vm-output.log"
export RUST_LOG="debug"

# With networking
export TAP_DEVICE="tap0"
export GUEST_IP="192.168.100.2"
export HOST_IP="192.168.100.1"
export NETMASK="255.255.255.0"
```

## Notes

- All path variables accept both relative and absolute paths
- Relative paths are resolved from the current working directory
- Network-related variables in VMM are optional and only required when TAP networking is enabled
- The agent's kernel path is critical for operation - ensure it points to a valid kernel image