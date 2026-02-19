// SPDX-License-Identifier: Apache-2.0

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};

pub const TUNSETIFF: u64 = 0x400454ca;
pub const TUNSETOWNER: u64 = 0x400454cc;
pub const IFF_TUN: u16 = 0x0001;
pub const IFF_TAP: u16 = 0x0002;
pub const IFF_NO_PI: u16 = 0x1000;

#[derive(Debug)]
pub struct TapDevice {
    fd: File,
    name: String,
}

impl TapDevice {
    pub fn new(name: &str) -> io::Result<Self> {
        let tun_path = "/dev/net/tun";

        let file = File::open(tun_path)?;

        let mut ifr_name = [0i8; 16];
        for (i, c) in name.as_bytes().iter().enumerate() {
            if i < 16 {
                ifr_name[i] = *c as i8;
            }
        }

        let mut request = TunRequest {
            ifr_name,
            ifr_flags: (IFF_TAP as i16 | IFF_NO_PI as i16),
        };

        unsafe {
            let ret = libc::ioctl(
                file.as_raw_fd(),
                TUNSETIFF as libc::c_ulong,
                &mut request as *mut _ as *mut libc::c_void,
            );
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        let mut actual_name = String::new();
        for c in &request.ifr_name {
            if *c != 0 {
                actual_name.push(*c as u8 as char);
            }
        }

        Ok(TapDevice {
            fd: file,
            name: actual_name,
        })
    }

    pub fn set_mtu(&self, mtu: u32) -> io::Result<()> {
        let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if sock < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut ifr_name = [0i8; 16];
        for (i, c) in self.name.as_bytes().iter().enumerate() {
            if i < 16 {
                ifr_name[i] = *c as i8;
            }
        }

        let mut ifr = IfreqMtu {
            ifr_name,
            ifr_mtu: mtu as libc::c_int,
        };

        unsafe {
            let ret = libc::ioctl(
                sock,
                libc::SIOCSIFMTU as libc::c_ulong,
                &mut ifr as *mut _ as *mut libc::c_void,
            );
            libc::close(sock);
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    pub fn set_ip_addr(&self, addr: &str, netmask: &str) -> io::Result<()> {
        let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if sock < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut ifr_name = [0i8; 16];
        for (i, c) in self.name.as_bytes().iter().enumerate() {
            if i < 16 {
                ifr_name[i] = *c as i8;
            }
        }

        let ip_parts: Vec<&str> = addr.split('.').collect();
        let mut sa_data = [0i8; 14];
        if ip_parts.len() == 4 {
            sa_data[0] = ip_parts[0].parse::<i8>().unwrap_or(0);
            sa_data[1] = ip_parts[1].parse::<i8>().unwrap_or(0);
            sa_data[2] = ip_parts[2].parse::<i8>().unwrap_or(0);
            sa_data[3] = ip_parts[3].parse::<i8>().unwrap_or(0);
        }

        let mut ifr_addr = IfreqAddr {
            ifr_name,
            ifr_addr: SockaddrIn {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: 0,
                sin_addr: InAddr { s_addr: 0 },
                sin_zero: [0; 8],
            },
        };

        unsafe {
            let ret = libc::ioctl(
                sock,
                libc::SIOCSIFADDR as libc::c_ulong,
                &mut ifr_addr as *mut _ as *mut libc::c_void,
            );
            if ret < 0 {
                libc::close(sock);
                return Err(io::Error::last_os_error());
            }
        }

        let nm_parts: Vec<&str> = netmask.split('.').collect();
        let mut nm_sa_data = [0i8; 14];
        if nm_parts.len() == 4 {
            nm_sa_data[0] = nm_parts[0].parse::<i8>().unwrap_or(0);
            nm_sa_data[1] = nm_parts[1].parse::<i8>().unwrap_or(0);
            nm_sa_data[2] = nm_parts[2].parse::<i8>().unwrap_or(0);
            nm_sa_data[3] = nm_parts[3].parse::<i8>().unwrap_or(0);
        }

        let mut ifr_netmask = IfreqAddr {
            ifr_name,
            ifr_addr: SockaddrIn {
                sin_family: libc::AF_INET as libc::sa_family_t,
                sin_port: 0,
                sin_addr: InAddr { s_addr: 0 },
                sin_zero: [0; 8],
            },
        };

        unsafe {
            let ret = libc::ioctl(
                sock,
                libc::SIOCSIFNETMASK as libc::c_ulong,
                &mut ifr_netmask as *mut _ as *mut libc::c_void,
            );
            libc::close(sock);
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    pub fn bring_up(&self) -> io::Result<()> {
        let sock = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
        if sock < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut ifr_name = [0i8; 16];
        for (i, c) in self.name.as_bytes().iter().enumerate() {
            if i < 16 {
                ifr_name[i] = *c as i8;
            }
        }

        let mut ifr = IfreqFlags {
            ifr_name,
            ifr_flags: (IFF_TAP as i16
                | IFF_NO_PI as i16
                | libc::IFF_UP as i16
                | libc::IFF_RUNNING as i16),
        };

        unsafe {
            let ret = libc::ioctl(
                sock,
                libc::SIOCSIFFLAGS as libc::c_ulong,
                &mut ifr as *mut _ as *mut libc::c_void,
            );
            libc::close(sock);
            if ret < 0 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Read for TapDevice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.fd.read(buf)
    }
}

impl Write for TapDevice {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.fd.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.fd.flush()
    }
}

impl AsRawFd for TapDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

#[repr(C)]
struct TunRequest {
    ifr_name: [i8; 16],
    ifr_flags: libc::c_short,
}

#[repr(C)]
struct IfreqMtu {
    ifr_name: [i8; 16],
    ifr_mtu: libc::c_int,
}

#[repr(C)]
struct IfreqAddr {
    ifr_name: [i8; 16],
    ifr_addr: SockaddrIn,
}

#[repr(C)]
struct IfreqFlags {
    ifr_name: [i8; 16],
    ifr_flags: libc::c_short,
}

#[repr(C)]
struct SockaddrIn {
    sin_family: libc::sa_family_t,
    sin_port: libc::in_port_t,
    sin_addr: InAddr,
    sin_zero: [i8; 8],
}

#[repr(C)]
struct InAddr {
    s_addr: libc::in_addr_t,
}
