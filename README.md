# Cloude

Serverless Rust program to run code from clients in micro-vm

## Table of contents

## Dependancies

Cloud is written in Rust and makes use of the following libraries :
- anyhow
- axum
- clap
- epoll
- event-manager
- futures-util
- initramfs-builder
- kvm-bindings
- kvm-ioctls
- libc
- linux-loader
- log
- nftables
- regex
- rtnetlink
- serde
- serde_json
- tokio
- tracing
- tracing-subscriber
- uuid
- virt
- vm-memory
- vmm-sys-util
- vm-device
- vm-superio
- vm-allocator
- virtio-queue
- virtio-device

## Architecture

Cloude is made up of 5 parts. They are assembled together to create the flow of the project :
- `initramfs-builder` handles the creation of the initramfs to start with a kernel to have the dependancies for the language we need in the VM
- `vmm` is the main crate that is used as a wrapper above KVM to make it easier to use by abstracting virtio support, serial channel handling and so on
- `backend` is a small API that takes HTTP requests with code to execute, starts the VM thanks to vmm crate and send the order to the agent in the VM
- `agent` take care of the execution of the code in the VM, it receives the order from the backend and return the output of the execution
- `cli` constitute the last brick that is the user interface and allow sending requests to backend through HTTP

In a more graphical way this is the architecture of the project :
        +------------------+
        |       CLI        |
        |  User Interface  |
        | Sends HTTP Req   |
        +--------+---------+
                |
                v
        +------------------+
        |     Backend      |
        |    HTTP API      |
        | Start VM + Send  |
        | Exec Requests    |
        +--------+---------+
                |
                v
        +------------------+
        |       VMM        |
        |  KVM Wrapper     |
        |                  |
        | VM Management    |
        +--------+---------+
                |
    Starts VM   |
                v
    +--------------------------+
    |           VM             |
    |                          |
    |  +--------------------+  |
    |  |       Agent        |  |
    |  | Execute Code       |  |
    |  | Return Output      |  |
    |  +---------+----------+  |
    |                          |
    |  +---------+----------+  |
    |  |   Initramfs Builder | |
    |  | Kernel + Runtime    | |
    |  | Dependencies        | |
    |  +--------------------+  |
    +--------------------------+

### Initramfs Builder

The initramfs builder handles all the process of creating the image that will be used to start the VM and customize it.
It will bring dependancies for languages

**Features :**
- 

### VMM

The VMM is a wrapper to KVM that handles the logic to manage the VM.
It brings methods to create VMs, handle virtio device creation, assign IP, manage serial channels.

**Features :**
- Programs can create VM with ressources allocated
- IP can be set to handle program-VM communication
- Serial port receives the output of the commands

### Backend

The backend is an API that receives HTTP requests with language and code to execute.
It will create a VM and send the order of execution before returning the output.

**Features :**
- User send requests to the specified route
- A VM is created with a specific IP, ressources and image
- The code is sent to the agent inside the VM for execution
- After execution the output is returned

### Agent

The agent is in charge of receiving code to execute, start it and wait for the complete execution.

**Features :**
- The code to execute is received from the backend
- The instructions are executed in the VM
- After execution the output is sent back to the backend

### CLI

The CLI is a user-friendly interface to interact with the backend without CURL requests.

**Features :**
- Commands are executed in the user space
- Requests goes to the designed backend
- Code output is received from the backend after execution

## Components lifetime

A micro-VM is spawned for every request as in serverless architecture.
That way the agent only exists for the time of execution while the backend is constantly listening for execution requests.

The CLI is only used when the user needs it.

## Licence

The project is distributed under license `Apache License 2.0`