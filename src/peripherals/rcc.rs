// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;
use super::meta::PeripheralMeta;
use super::{Peripheral, PlatformFamily};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RccLayout {
    F1,
    F4,
}

pub struct Rcc {
    layout: RccLayout,
    offsets: RccOffsets,
    cr: u32,
    pllcfgr: u32,
    cfgr: u32,
    cir: u32,
    ahb1rstr: u32,
    apb2rstr: u32,
    apb1rstr: u32,
    ahbenr: u32,
    ahb1enr: u32,
    apb2enr: u32,
    apb1enr: u32,
    bdcr: u32,
    csr: u32,
}

#[derive(Clone, Copy)]
struct RccOffsets {
    cr: u32,
    pllcfgr: u32,
    cfgr: u32,
    cir: u32,
    ahb1rstr: u32,
    apb2rstr: u32,
    apb1rstr: u32,
    ahbenr: u32,
    ahb1enr: u32,
    apb2enr: u32,
    apb1enr: u32,
    bdcr: u32,
    csr: u32,
}

impl Rcc {
    pub fn new(name: &str, platform: PlatformFamily, meta: Option<&PeripheralMeta>) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" {
            let layout = Self::detect_layout(platform, meta);
            let offsets = Self::offsets(layout, meta);
            Some(Box::new(Rcc {
                layout,
                offsets,
                // HSI on by default after reset.
                cr: 1 << 0,
                // STM32F4 reset value; ignored on F1.
                pllcfgr: 0x2400_3010,
                cfgr: 0,
                cir: 0,
                ahb1rstr: 0,
                apb2rstr: 0,
                apb1rstr: 0,
                ahbenr: 0,
                ahb1enr: 0,
                apb2enr: 0,
                apb1enr: 0,
                bdcr: 0,
                csr: 0,
            }))
        } else {
            None
        }
    }

    fn detect_layout(platform: PlatformFamily, meta: Option<&PeripheralMeta>) -> RccLayout {
        if let Some(meta) = meta {
            if meta.has_register("PLLCFGR") || meta.has_register("AHB1ENR") {
                return RccLayout::F4;
            }
            if meta.has_register("AHBENR") {
                return RccLayout::F1;
            }
        }

        match platform {
            PlatformFamily::Stm32F1 => RccLayout::F1,
            PlatformFamily::Stm32F4 | PlatformFamily::Auto => RccLayout::F4,
        }
    }

    fn offsets(layout: RccLayout, meta: Option<&PeripheralMeta>) -> RccOffsets {
        let reg = |name: &str, default: u32| meta.and_then(|m| m.offset_of(name)).unwrap_or(default);
        match layout {
            RccLayout::F1 => RccOffsets {
                cr: reg("CR", 0x0000),
                pllcfgr: reg("PLLCFGR", 0x0004),
                cfgr: reg("CFGR", 0x0004),
                cir: reg("CIR", 0x0008),
                apb2rstr: reg("APB2RSTR", 0x000C),
                apb1rstr: reg("APB1RSTR", 0x0010),
                ahbenr: reg("AHBENR", 0x0014),
                apb2enr: reg("APB2ENR", 0x0018),
                apb1enr: reg("APB1ENR", 0x001C),
                bdcr: reg("BDCR", 0x0020),
                csr: reg("CSR", 0x0024),
                ahb1rstr: reg("AHB1RSTR", 0x0010),
                ahb1enr: reg("AHB1ENR", 0x0030),
            },
            RccLayout::F4 => RccOffsets {
                cr: reg("CR", 0x0000),
                pllcfgr: reg("PLLCFGR", 0x0004),
                cfgr: reg("CFGR", 0x0008),
                cir: reg("CIR", 0x000C),
                ahb1rstr: reg("AHB1RSTR", 0x0010),
                apb1rstr: reg("APB1RSTR", 0x0020),
                apb2rstr: reg("APB2RSTR", 0x0024),
                ahb1enr: reg("AHB1ENR", 0x0030),
                apb1enr: reg("APB1ENR", 0x0040),
                apb2enr: reg("APB2ENR", 0x0044),
                bdcr: reg("BDCR", 0x0070),
                csr: reg("CSR", 0x0074),
                ahbenr: reg("AHBENR", 0x0014),
            },
        }
    }

    fn update_ready_bits(&mut self) {
        // CR ready bits.
        if self.cr & (1 << 0) != 0 { self.cr |= 1 << 1; }  else { self.cr &= !(1 << 1); }   // HSIRDY
        if self.cr & (1 << 16) != 0 { self.cr |= 1 << 17; } else { self.cr &= !(1 << 17); }  // HSERDY
        if self.cr & (1 << 24) != 0 { self.cr |= 1 << 25; } else { self.cr &= !(1 << 25); }  // PLLRDY

        // CSR ready bits.
        if self.csr & (1 << 0) != 0 { self.csr |= 1 << 1; } else { self.csr &= !(1 << 1); }  // LSIRDY

        // CFGR system clock switch status (SWS) follows requested SW.
        let sw = self.cfgr & 0b11;
        self.cfgr = (self.cfgr & !(0b11 << 2)) | (sw << 2);
    }
}


impl Peripheral for Rcc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        self.update_ready_bits();
        if offset == self.offsets.cr {
            self.cr
        } else if self.layout == RccLayout::F4 && offset == self.offsets.pllcfgr {
            self.pllcfgr
        } else if offset == self.offsets.cfgr {
            self.cfgr
        } else if offset == self.offsets.cir {
            self.cir
        } else if self.layout == RccLayout::F4 && offset == self.offsets.ahb1rstr {
            self.ahb1rstr
        } else if offset == self.offsets.apb1rstr {
            self.apb1rstr
        } else if offset == self.offsets.apb2rstr {
            self.apb2rstr
        } else if self.layout == RccLayout::F1 && offset == self.offsets.ahbenr {
            self.ahbenr
        } else if self.layout == RccLayout::F4 && offset == self.offsets.ahb1enr {
            self.ahb1enr
        } else if offset == self.offsets.apb1enr {
            self.apb1enr
        } else if offset == self.offsets.apb2enr {
            self.apb2enr
        } else if offset == self.offsets.bdcr {
            self.bdcr
        } else if offset == self.offsets.csr {
            self.csr
        } else {
            0
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        if offset == self.offsets.cr {
            self.cr = value;
        } else if self.layout == RccLayout::F4 && offset == self.offsets.pllcfgr {
            self.pllcfgr = value;
        } else if offset == self.offsets.cfgr {
            self.cfgr = value;
        } else if offset == self.offsets.cir {
            self.cir = value;
        } else if self.layout == RccLayout::F4 && offset == self.offsets.ahb1rstr {
            self.ahb1rstr = value;
        } else if offset == self.offsets.apb1rstr {
            self.apb1rstr = value;
        } else if offset == self.offsets.apb2rstr {
            self.apb2rstr = value;
        } else if self.layout == RccLayout::F1 && offset == self.offsets.ahbenr {
            self.ahbenr = value;
        } else if self.layout == RccLayout::F4 && offset == self.offsets.ahb1enr {
            self.ahb1enr = value;
        } else if offset == self.offsets.apb1enr {
            self.apb1enr = value;
        } else if offset == self.offsets.apb2enr {
            self.apb2enr = value;
        } else if offset == self.offsets.bdcr {
            self.bdcr = value;
        } else if offset == self.offsets.csr {
            self.csr = value;
        }
        self.update_ready_bits();
    }
}
