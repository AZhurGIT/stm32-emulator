// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::Result;
use serde::Deserialize;
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;

use crate::system::System;

use super::ExtDevice;

#[derive(Debug, Deserialize)]
pub struct UsartProbeConfig {
    pub peripheral: String,
    pub bridge_tty: Option<String>,
    /// Line speed for the host TTY. Use the same value as the firmware USART (e.g. 115200 or 200000).
    /// Non-standard rates need Linux `termios2` + `BOTHER` (implemented below).
    #[serde(default = "default_bridge_baud")]
    pub bridge_baud: u32,
}

fn default_bridge_baud() -> u32 {
    115200
}

impl Default for UsartProbeConfig {
    fn default() -> Self {
        Self {
            peripheral: String::new(),
            bridge_tty: None,
            bridge_baud: 115200,
        }
    }
}

#[derive(Default)]
pub struct UsartProbe {
    pub config: UsartProbeConfig,
    name: String,
    trace_enabled: bool,
    rx: Vec<u8>,
    bridge: Option<SerialBridge>,
}

impl UsartProbe {
    pub fn new(config: UsartProbeConfig) -> Result<Self> {
        let bridge = if let Some(path) = config.bridge_tty.as_ref() {
            info!(
                "USART bridge {} -> {} baud",
                path, config.bridge_baud
            );
            Some(SerialBridge::open(path, config.bridge_baud)?)
        } else {
            None
        };
        Ok(Self { config, bridge, trace_enabled: true, ..Self::default() })
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
    }
}

impl ExtDevice<(), u8> for UsartProbe {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} usart-probe", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        self.bridge
            .as_ref()
            .and_then(|bridge| bridge.rx_queue.lock().ok().and_then(|mut q| q.pop_front()))
            .unwrap_or(0)
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if let Some(ref mut bridge) = self.bridge {
            bridge.write_byte(v);
        }

        if v == 0x0a {
            // EOL
            let line = String::from_utf8_lossy(&self.rx);
            let line = line.trim();
            if self.trace_enabled {
                info!("{} '{}'", self.name, line);
            }
            self.rx.clear();
        } else {
            self.rx.push(v);
        }
    }

    fn available(&self) -> usize {
        self.bridge
            .as_ref()
            .and_then(|bridge| bridge.rx_queue.lock().ok().map(|q| q.len()))
            .unwrap_or(0)
    }
}

struct SerialBridge {
    tx: File,
    rx_queue: Arc<Mutex<VecDeque<u8>>>,
}

impl SerialBridge {
    fn open(path: &str, baud: u32) -> Result<Self> {
        let mut opts = OpenOptions::new();
        opts.read(true).write(true);
        #[cfg(unix)]
        opts.custom_flags(libc::O_NOCTTY);
        let file = opts.open(path)?;
        #[cfg(unix)]
        configure_host_serial(&file, baud)?;
        let reader = file.try_clone()?;
        let rx_queue = Arc::new(Mutex::new(VecDeque::new()));
        let rx_queue_worker = rx_queue.clone();
        let path = path.to_string();
        let path_for_thread = path.clone();

        thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 256];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => thread::sleep(Duration::from_millis(10)),
                    Ok(n) => {
                        if let Ok(mut q) = rx_queue_worker.lock() {
                            q.extend(&buf[..n]);
                        }
                    }
                    Err(e) => {
                        warn!("USART bridge read error path={} err={}", path_for_thread, e);
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
        });

        info!("USART bridge connected to {}", path);

        Ok(Self { tx: file, rx_queue })
    }

    fn write_byte(&mut self, b: u8) {
        if let Err(e) = self.tx.write_all(&[b]) {
            warn!("USART bridge write error err={}", e);
            return;
        }
        if let Err(e) = self.tx.flush() {
            warn!("USART bridge flush error err={}", e);
        }
    }
}

#[cfg(unix)]
fn configure_host_serial(file: &File, baud: u32) -> Result<()> {
    use std::os::unix::io::AsRawFd;

    let fd = file.as_raw_fd();

    #[cfg(target_os = "linux")]
    match configure_linux_termios2(fd, baud) {
        Ok(()) => {
            info!("USART bridge TTY raw {} baud (termios2/BOTHER)", baud);
            return Ok(());
        }
        Err(e) => {
            warn!(
                "termios2 failed ({}); falling back to legacy termios — non-standard baud may be wrong",
                e
            );
        }
    }

    configure_unix_termios_legacy(fd, baud)?;
    info!("USART bridge TTY raw {} baud (legacy termios)", baud);
    Ok(())
}

/// Linux: arbitrary baud (e.g. 200000) via `termios2` + `BOTHER` (`TCGETS2` / `TCSETS2`).
#[cfg(target_os = "linux")]
fn configure_linux_termios2(fd: libc::c_int, baud: u32) -> Result<()> {
    use std::mem::MaybeUninit;

    unsafe {
        let mut t2 = MaybeUninit::<libc::termios2>::uninit();
        if libc::ioctl(fd, libc::TCGETS2, t2.as_mut_ptr()) != 0 {
            return Err(anyhow::anyhow!("TCGETS2: {}", std::io::Error::last_os_error()));
        }
        let mut t2 = t2.assume_init();

        t2.c_iflag &= !(libc::IGNBRK
            | libc::BRKINT
            | libc::PARMRK
            | libc::ISTRIP
            | libc::INLCR
            | libc::IGNCR
            | libc::ICRNL
            | libc::IXON);
        t2.c_oflag &= !libc::OPOST;
        t2.c_lflag &=
            !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
        t2.c_cflag &= !(libc::CSIZE | libc::PARENB);
        t2.c_cflag |= libc::CS8 | libc::CREAD | libc::CLOCAL;
        t2.c_cflag &= !libc::CSTOPB;

        t2.c_cflag &= !(libc::CBAUD | libc::CBAUDEX);
        t2.c_cflag |= libc::BOTHER;
        t2.c_ispeed = baud as libc::speed_t;
        t2.c_ospeed = baud as libc::speed_t;

        if libc::ioctl(fd, libc::TCSETS2, &mut t2 as *mut _) != 0 {
            anyhow::bail!("TCSETS2: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn configure_unix_termios_legacy(fd: libc::c_int, baud: u32) -> Result<()> {
    let speed = baud_to_libc_speed(baud);
    if speed == libc::B115200 && !matches!(baud, 115200) {
        warn!(
            "USART bridge: baud {} has no exact B* constant; using B115200 — set Linux termios2 or pick a standard rate",
            baud
        );
    }
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) != 0 {
            anyhow::bail!("tcgetattr failed: {}", std::io::Error::last_os_error());
        }
        libc::cfmakeraw(&mut t);
        if libc::cfsetispeed(&mut t, speed) != 0 || libc::cfsetospeed(&mut t, speed) != 0 {
            anyhow::bail!("cfsetspeed failed: {}", std::io::Error::last_os_error());
        }
        t.c_cflag |= libc::CREAD | libc::CLOCAL;
        t.c_cflag &= !libc::CSTOPB;
        t.c_cflag &= !libc::PARENB;
        t.c_cflag = (t.c_cflag & !libc::CSIZE) | libc::CS8;
        if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
            anyhow::bail!("tcsetattr failed: {}", std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(unix)]
fn baud_to_libc_speed(baud: u32) -> libc::speed_t {
    match baud {
        9600 => libc::B9600,
        19200 => libc::B19200,
        38400 => libc::B38400,
        57600 => libc::B57600,
        115200 => libc::B115200,
        230400 => libc::B230400,
        460800 => libc::B460800,
        921600 => libc::B921600,
        _ => libc::B115200,
    }
}
