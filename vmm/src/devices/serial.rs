// SPDX-License-Identifier: Apache-2.0

use std::fs::{File, OpenOptions};
use std::io::{stdout, Error, Result, Write};
use std::ops::Deref;
use std::path::Path;

use vm_superio::serial::NoEvents;
use vm_superio::{Serial, Trigger};
use vmm_sys_util::eventfd::EventFd;

pub const SERIAL_PORT_BASE: u16 = 0x3f8;
pub const SERIAL_PORT_LAST: u16 = 0x3ff;

pub struct EventFdTrigger(EventFd);

impl Trigger for EventFdTrigger {
    type E = Error;

    fn trigger(&self) -> Result<()> {
        self.write(1)
    }
}

impl Deref for EventFdTrigger {
    type Target = EventFd;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl EventFdTrigger {
    pub fn new(flag: i32) -> Result<Self> {
        Ok(EventFdTrigger(EventFd::new(flag)?))
    }
    pub fn try_clone(&self) -> Result<Self> {
        Ok(EventFdTrigger((**self).try_clone()?))
    }
}

pub(crate) struct LumperSerial {
    // evenfd allows for the device to send interrupts to the guest.
    eventfd: EventFdTrigger,

    // serial is the actual serial device.
    pub serial: Serial<EventFdTrigger, NoEvents, Box<dyn std::io::Write + Send>>,
}

impl LumperSerial {
    pub fn new(output: Box<dyn std::io::Write + Send>) -> Result<Self> {
        let eventfd = EventFdTrigger::new(libc::EFD_NONBLOCK).unwrap();

        Ok(LumperSerial {
            eventfd: eventfd.try_clone()?,
            serial: Serial::new(eventfd.try_clone()?, output),
        })
    }

    pub fn eventfd(&self) -> Result<EventFd> {
        Ok(self.eventfd.try_clone()?.0)
    }
}
