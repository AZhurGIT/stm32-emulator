// SPDX-License-Identifier: GPL-3.0-or-later

use crate::util::UniErr;
use crate::system::System;
use super::Peripheral;
use super::Peripherals;
use super::PlatformFamily;

pub struct Dma {
    name: String,
    layout: DmaLayout,
    streams: [Stream; 8],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DmaLayout {
    Auto,
    F1,
    F4,
}

impl Dma {
    pub fn new(name: &str, platform: PlatformFamily) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("DMA") {
            let name = name.to_string();
            let layout = match platform {
                PlatformFamily::Stm32F1 => DmaLayout::F1,
                PlatformFamily::Stm32F4 => DmaLayout::F4,
                PlatformFamily::Auto => DmaLayout::Auto,
            };
            Some(Box::new(Self { name, layout, streams: Default::default() }))
        } else {
            None
        }
    }

    // STM32F1 DMA layout: CH1 at 0x08, stride 0x14, regs: CCR/CNDTR/CPAR/CMAR.
    fn f1_channel_access(offset: u32) -> Option<(usize, u32)> {
        if !(0x08..0x94).contains(&offset) {
            return None;
        }

        let rel = offset - 0x08;
        let channel = (rel / 0x14) as usize;
        let ch_offset = rel % 0x14;
        if channel < 7 && ch_offset <= 0x0C {
            Some((channel, ch_offset))
        } else {
            None
        }
    }
}

impl Peripheral for Dma {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        if self.layout != DmaLayout::F4 {
            if let Some((i, ch_offset)) = Self::f1_channel_access(offset) {
                return self.streams[i].read_f1(&self.name, sys, ch_offset);
            }
        }

        if self.layout == DmaLayout::F1 {
            return 0;
        }

        match Access::from_offset(offset) {
            Access::StreamReg(i, offset) => self.streams[i].read(&self.name, sys, offset),
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        if self.layout != DmaLayout::F4 {
            if let Some((i, ch_offset)) = Self::f1_channel_access(offset) {
                self.streams[i].write_f1(&self.name, sys, ch_offset, value);
                return;
            }
        }

        if self.layout == DmaLayout::F1 {
            return;
        }

        match Access::from_offset(offset) {
            Access::StreamReg(i, offset) => self.streams[i].write(&self.name, sys, offset, value),
            _ => {}
        }
    }
}

#[derive(Default)]
struct Stream {
    pub cr: u32,
    pub next_cr: Option<u32>,
    pub ndtr: u32,
    pub par: u32,
    pub m0ar: u32,
    pub m1ar: u32,
    pub fcr: u32,
    // STM32F1 runtime state for event-driven peripheral->memory channels.
    f1_reload_ndtr: u32,
    f1_mem_addr: u32,
}

impl Stream {
    fn channel(&self) -> u8 {
        ((self.cr >> 25) & 0b111) as u8
    }

    fn dir(&self) -> Dir {
        match (self.cr >> 6) & 0b11 {
            0b00 => Dir::Read,
            0b01 => Dir::Write,
            0b10 => Dir::MemCopy,
            _ => Dir::Invalid,
        }
    }

    // 1, 2, 4 (8bit, 16bit, 32bit)
    fn word_size(&self) -> usize {
        match (self.cr >> 11) & 0b11 {
            0b00 => 1,
            0b01 => 2,
            0b10 => 4,
            _ => 1,
        }
    }

    fn data_size(&self) -> usize {
        self.word_size() * self.ndtr as usize
    }

    // STM32F1: memory size is in bits 11:10 in CCRx.
    fn word_size_f1(&self) -> usize {
        match (self.cr >> 10) & 0b11 {
            0b00 => 1,
            0b01 => 2,
            0b10 => 4,
            _ => 1,
        }
    }

    fn data_addr(&self) -> u32 {
        if (self.cr >> 19) & 1 != 0 {
            self.m1ar
        } else {
            self.m0ar
        }
    }

    fn do_xfer(&self, name: &str, sys: &System) {
        let dir = self.dir();
        let data_addr = self.data_addr();
        let size = self.data_size();
        let peri_addr = self.par;

        let peri = Peripherals::get_peripheral(&sys.p.peripherals, peri_addr);

        let (src, dst) = match dir {
            Dir::Read => (peri_addr, data_addr),
            Dir::Write => (data_addr, peri_addr),
            Dir::MemCopy => (peri_addr, data_addr),
            Dir::Invalid => (0,0),
        };

        if log::log_enabled!(log::Level::Debug) {
            let peri_desc = sys.p.addr_desc(peri_addr);
            debug!("{} xfer initiated channel={} peri_{} dir={:?} addr=0x{:08x} size={}",
                name, self.channel(), peri_desc, dir, data_addr, size);
        }

        let buf = match dir {
            Dir::Read => {
                peri.map(|p| p.peripheral.borrow_mut().read_dma(sys, peri_addr-p.start, size))
            }
            Dir::Write | Dir::MemCopy => {
                sys.uc.borrow().mem_read_as_vec(src.into(), size)
                    .map_err(|e| warn!("DMA read failed addr=0x{:08x} size={} e={}", src, size, UniErr(e)))
                    .map(|v| v.into())
                    .ok()
            }
            Dir::Invalid => Some(vec![].into()),
        };

        let mut buf = buf.unwrap_or_else(|| {
            let mut rx = vec![];
            rx.resize(size, 0);
            rx.into()
        });

        trace!("{} xfer buf={:x?}", name, buf);

        match dir {
            Dir::Write => {
                peri.map(|p| p.peripheral.borrow_mut().write_dma(sys, peri_addr-p.start, buf));
            }
            Dir::Read | Dir::MemCopy => {
                if let Err(e) = sys.uc.borrow_mut().mem_write(dst.into(), buf.make_contiguous()) {
                    warn!("DMA read failed addr=0x{:08x} size={} e={}", dst, size, UniErr(e));
                }
            }
            Dir::Invalid => {}
        }
    }

    pub fn read(&mut self, _name: &str, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => {
                let v = self.cr;
                if let Some(next_cr) = self.next_cr.take() {
                    self.cr = next_cr;
                }

                // The saturn firmware is a bit buggy. When doing a DMA write
                // with size=0, they don't enable the DMA channel, but they
                // wait for it to go to 1 and then 0, with a timeout. So they
                // are consistently hitting the timeout.
                // We'll do toggles on the ready flag to speed things up avoiding the timeout.
                if self.dir() == Dir::Write && self.data_size() == 0 {
                    self.next_cr = Some(self.cr ^ 1)
                }

                v
            }
            0x0004 => self.ndtr,
            0x0008 => self.par,
            0x000c => self.m0ar,
            0x0010 => self.m1ar,
            0x0014 => self.fcr,
            _ => 0
        }
    }

    pub fn write(&mut self, name: &str, sys: &System, offset: u32, mut value: u32) {
        match offset {
            0x0000 => {
                self.cr = value;

                // CRx register
                if value & 1 != 0 {
                    // Enable is on. do the transfer.
                    self.do_xfer(name, sys);

                    value &= !1;
                    self.ndtr = 0;
                    self.next_cr = Some(value);
                }
            }
            0x0004 => { self.ndtr = value & 0xFFFF; }
            0x0008 => { self.par = value; }
            0x000c => { self.m0ar = value; }
            0x0010 => { self.m1ar = value; }
            0x0014 => { self.fcr = value; }
            _ => {}
        }
    }

    pub fn read_f1(&mut self, name: &str, sys: &System, offset: u32) -> u32 {
        // For F1 P2M channels, transfer progresses on peripheral events.
        // We emulate this lazily when firmware polls DMA registers.
        self.pump_f1_rx(name, sys);

        match offset {
            0x0000 => self.cr,
            0x0004 => self.ndtr,
            0x0008 => self.par,
            0x000c => self.m0ar,
            _ => 0,
        }
    }

    pub fn write_f1(&mut self, name: &str, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => {
                self.cr = value;
                // EN bit.
                if value & 1 != 0 {
                    if self.f1_reload_ndtr == 0 {
                        self.f1_reload_ndtr = self.ndtr;
                    }
                    self.f1_mem_addr = self.m0ar;

                    // For RX channels (P2M), DMA should wait for peripheral events.
                    // For TX channels (M2P), we can execute immediately.
                    let mem_to_peri = (self.cr >> 4) & 1 != 0;
                    if mem_to_peri {
                        self.do_xfer_f1(name, sys);
                        self.cr &= !1;
                        self.ndtr = 0;
                    }
                }
            }
            0x0004 => {
                self.ndtr = value & 0xFFFF;
                if self.cr & 1 == 0 {
                    self.f1_reload_ndtr = self.ndtr;
                }
            }
            0x0008 => self.par = value,
            0x000c => {
                self.m0ar = value;
                if self.cr & 1 == 0 {
                    self.f1_mem_addr = value;
                }
            }
            _ => {}
        }
    }

    fn pump_f1_rx(&mut self, name: &str, sys: &System) {
        // EN=1 and DIR=0 => peripheral-to-memory channel (e.g. USARTx_RX).
        if (self.cr & 1) == 0 || ((self.cr >> 4) & 1) != 0 {
            return;
        }

        if self.ndtr == 0 {
            let circular = ((self.cr >> 5) & 1) != 0;
            if circular && self.f1_reload_ndtr != 0 {
                self.ndtr = self.f1_reload_ndtr;
                self.f1_mem_addr = self.m0ar;
            } else {
                self.cr &= !1;
                return;
            }
        }

        let word_size = self.word_size_f1();
        let max_bytes = self.ndtr as usize * word_size;
        if max_bytes == 0 {
            return;
        }

        let peri_addr = self.par;
        let peri = Peripherals::get_peripheral(&sys.p.peripherals, peri_addr);
        let mut buf = peri
            .map(|p| p.peripheral.borrow_mut().read_dma(sys, peri_addr - p.start, max_bytes))
            .unwrap_or_default();

        if buf.is_empty() {
            return;
        }

        let units = (buf.len() / word_size).min(self.ndtr as usize);
        let bytes = units * word_size;
        if bytes == 0 {
            return;
        }
        buf.truncate(bytes);

        let mem_inc = ((self.cr >> 7) & 1) != 0;
        let dst = if mem_inc { self.f1_mem_addr } else { self.m0ar };
        if let Err(e) = sys.uc.borrow_mut().mem_write(dst.into(), buf.make_contiguous()) {
            warn!(
                "{} DMA(F1) RX write failed addr=0x{:08x} size={} e={}",
                name,
                dst,
                bytes,
                UniErr(e)
            );
            return;
        }

        if mem_inc {
            self.f1_mem_addr = self.f1_mem_addr.wrapping_add(bytes as u32);
        }
        self.ndtr = self.ndtr.saturating_sub(units as u32);

        if self.ndtr == 0 {
            let circular = ((self.cr >> 5) & 1) != 0;
            if circular && self.f1_reload_ndtr != 0 {
                self.ndtr = self.f1_reload_ndtr;
                self.f1_mem_addr = self.m0ar;
            } else {
                self.cr &= !1;
            }
        }
    }

    fn do_xfer_f1(&self, name: &str, sys: &System) {
        // CCR DIR bit (bit4): 0 peripheral->memory, 1 memory->peripheral.
        let mem_to_peri = (self.cr >> 4) & 1 != 0;
        let size = self.word_size_f1() * self.ndtr as usize;
        let peri_addr = self.par;
        let mem_addr = self.m0ar;
        let peri = Peripherals::get_peripheral(&sys.p.peripherals, peri_addr);

        if log::log_enabled!(log::Level::Debug) {
            let peri_desc = sys.p.addr_desc(peri_addr);
            debug!(
                "{} xfer(F1) peri_{} dir={} mem=0x{:08x} size={}",
                name,
                peri_desc,
                if mem_to_peri { "M2P" } else { "P2M" },
                mem_addr,
                size
            );
        }

        if mem_to_peri {
            let buf = sys.uc.borrow().mem_read_as_vec(mem_addr.into(), size)
                .map_err(|e| warn!("DMA(F1) read failed addr=0x{:08x} size={} e={}", mem_addr, size, UniErr(e)))
                .unwrap_or_default();

            peri.map(|p| p.peripheral.borrow_mut().write_dma(sys, peri_addr - p.start, buf.into()));
        } else {
            let mut buf = peri
                .map(|p| p.peripheral.borrow_mut().read_dma(sys, peri_addr - p.start, size))
                .unwrap_or_default();

            if let Err(e) = sys.uc.borrow_mut().mem_write(mem_addr.into(), buf.make_contiguous()) {
                warn!("DMA(F1) write failed addr=0x{:08x} size={} e={}", mem_addr, size, UniErr(e));
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Dir {
    Read,
    Write,
    MemCopy,
    Invalid,
}

enum Access {
    Reg(u32),
    /// CR0, CR1, etc.
    StreamReg(usize, u32),
}

impl Access {
    pub fn from_offset(offset: u32) -> Self {
        if offset < 0x28 {
            Access::Reg(offset)
        } else {
            let stride = 0x18;
            let start = 0x10;

            let offset = offset - start;
            Access::StreamReg(
                (offset / stride) as usize,
                offset % stride
            )
        }
    }
}
