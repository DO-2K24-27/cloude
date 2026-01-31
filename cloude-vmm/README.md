# cloude-vmm

## Purpose

The `cloude-vmm` crate is responsible for managing and running virtual machines (VMs). It provides functionality to configure VM parameters such as CPU cores and memory size, load the kernel image, and start the VM execution.

## How to Use

To use `cloude-vmm`, ensure you have the necessary environment variable `KERNEL_PATH` set to the path of the kernel image you wish to load into the VM.

Run the VMM with the following command:

```bash
cargo run --bin cloude-vmm
```
