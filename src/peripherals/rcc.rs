// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;
use super::{Peripheral, PlatformFamily};

#[derive(Clone, Copy, PartialEq, Eq)]
enum RccLayout {
    F1,
    F4,
}

pub struct Rcc {
    layout: RccLayout,
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
    pub fn new(name: &str, platform: PlatformFamily) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" {
            let layout = match platform {
                PlatformFamily::Stm32F1 => RccLayout::F1,
                PlatformFamily::Stm32F4 | PlatformFamily::Auto => RccLayout::F4,
            };
            Some(Box::new(Rcc {
                layout,
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
        match self.layout {
            RccLayout::F1 => match offset {
                0x0000 => self.cr,
                0x0004 => self.cfgr,
                0x0008 => self.cir,
                0x000C => self.apb2rstr,
                0x0010 => self.apb1rstr,
                0x0014 => self.ahbenr,
                0x0018 => self.apb2enr,
                0x001C => self.apb1enr,
                0x0020 => self.bdcr,
                0x0024 => self.csr,
                _ => 0,
            },
            RccLayout::F4 => match offset {
                0x0000 => self.cr,
                0x0004 => self.pllcfgr,
                0x0008 => self.cfgr,
                0x000C => self.cir,
                0x0010 => self.ahb1rstr,
                0x0020 => self.apb1rstr,
                0x0024 => self.apb2rstr,
                0x0030 => self.ahb1enr,
                0x0040 => self.apb1enr,
                0x0044 => self.apb2enr,
                0x0070 => self.bdcr,
                0x0074 => self.csr,
                _ => 0,
            },
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match self.layout {
            RccLayout::F1 => match offset {
                0x0000 => self.cr = value,
                0x0004 => self.cfgr = value,
                0x0008 => self.cir = value,
                0x000C => self.apb2rstr = value,
                0x0010 => self.apb1rstr = value,
                0x0014 => self.ahbenr = value,
                0x0018 => self.apb2enr = value,
                0x001C => self.apb1enr = value,
                0x0020 => self.bdcr = value,
                0x0024 => self.csr = value,
                _ => {}
            },
            RccLayout::F4 => match offset {
                0x0000 => self.cr = value,
                0x0004 => self.pllcfgr = value,
                0x0008 => self.cfgr = value,
                0x000C => self.cir = value,
                0x0010 => self.ahb1rstr = value,
                0x0020 => self.apb1rstr = value,
                0x0024 => self.apb2rstr = value,
                0x0030 => self.ahb1enr = value,
                0x0040 => self.apb1enr = value,
                0x0044 => self.apb2enr = value,
                0x0070 => self.bdcr = value,
                0x0074 => self.csr = value,
                _ => {}
            },
        }
        self.update_ready_bits();
    }
}
