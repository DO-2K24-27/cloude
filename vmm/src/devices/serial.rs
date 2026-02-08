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

/// Writer that can write to both stdout and a file simultaneously
pub struct MultiWriter {
    file: Option<File>,
    stdout: bool,
}

impl MultiWriter {
    pub fn new(file_path: Option<&Path>, use_stdout: bool) -> Result<Self> {
        let file = if let Some(path) = file_path {
            Some(
                OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(path)?,
            )
        } else {
            None
        };

        Ok(MultiWriter {
            file,
            stdout: use_stdout,
        })
    }
}

impl Write for MultiWriter {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let mut written = 0;
        
        if self.stdout {
            written = stdout().write(buf)?;
        }
        
        if let Some(ref mut file) = self.file {
            file.write_all(buf)?;
            file.flush()?;
            written = buf.len();
        }
        
        Ok(written)
    }

    fn flush(&mut self) -> Result<()> {
        if self.stdout {
            stdout().flush()?;
        }
        
        if let Some(ref mut file) = self.file {
            file.flush()?;
        }
        
        Ok(())
    }
}

pub(crate) struct LumperSerial {
    // evenfd allows for the device to send interrupts to the guest.
    eventfd: EventFdTrigger,

    // serial is the actual serial device.
    pub serial: Serial<EventFdTrigger, NoEvents, MultiWriter>,
}

impl LumperSerial {
    pub fn new(output_path: Option<&Path>, use_stdout: bool) -> Result<Self> {
        let eventfd = EventFdTrigger::new(libc::EFD_NONBLOCK).unwrap();
        let writer = MultiWriter::new(output_path, use_stdout)?;

        Ok(LumperSerial {
            eventfd: eventfd.try_clone()?,
            serial: Serial::new(eventfd.try_clone()?, writer),
        })
    }

    pub fn eventfd(&self) -> Result<EventFd> {
        Ok(self.eventfd.try_clone()?.0)
    }
}
