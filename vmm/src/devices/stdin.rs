// SPDX-License-Identifier: Apache-2.0 OR BSD-3-Clause

use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use event_manager::{EventOps, Events, MutEventSubscriber};
use vmm_sys_util::epoll::EventSet;

use crate::devices::serial::LumperSerial;
use crate::VMInput;

const STDIN_DATA: u32 = 0;

struct FdWrapper(i32);

impl AsRawFd for FdWrapper {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0
    }
}

pub struct StdinHandler {
    input: Box<dyn VMInput>,
    serial: Arc<Mutex<LumperSerial>>,
    stdin_fd: Option<FdWrapper>,
}

impl StdinHandler {
    pub fn new(input: Box<dyn VMInput>, serial: Arc<Mutex<LumperSerial>>) -> Self {
        StdinHandler {
            input,
            serial,
            stdin_fd: None,
        }
    }
}

impl MutEventSubscriber for StdinHandler {
    fn process(&mut self, events: Events, ops: &mut EventOps) {
        if events.event_set() != EventSet::IN {
            return;
        }

        match events.data() {
            STDIN_DATA => {
                let mut out = [0u8; 64];
                match self.input.read(&mut out) {
                    Ok(n) if n > 0 => {
                        if let Err(e) = self
                            .serial
                            .lock()
                            .unwrap()
                            .serial
                            .enqueue_raw_bytes(&out[..n])
                        {
                            eprintln!("Failed to enqueue stdin bytes: {:?}", e);
                        }
                    }
                    Ok(0) => {
                        if let Some(fd) = &self.stdin_fd {
                            ops.remove(Events::empty(fd))
                                .expect("Failed to remove stdin event on EOF");
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to read stdin: {:?}", e);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn init(&mut self, ops: &mut EventOps) {
        let raw_fd = self.input.as_raw_fd();
        self.stdin_fd = Some(FdWrapper(raw_fd));

        ops.add(Events::with_data(
            self.stdin_fd.as_ref().unwrap(),
            STDIN_DATA,
            EventSet::IN,
        ))
        .expect("Unable to add stdin event");
    }
}
