// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;
use super::Peripheral;

#[derive(Default)]
pub struct Flash {
    name: String,
    acr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    ar: u32,
    obr: u32,
    wrpr: u32,
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FLASH" {
            Some(Box::new(Self {
                name: name.to_string(),
                // F1 reset value: LATENCY=0, HLFCYA=0, PRFTBE=0, PRFTBS=0.
                acr: 0,
                ..Default::default()
            }))
        } else {
            None
        }
    }

    fn sync_acr_status_bits(&mut self) {
        // F1: PRFTBS mirrors prefetch enabled state in this emulator model.
        const PRFTBE: u32 = 1 << 4;
        const PRFTBS: u32 = 1 << 5;
        if self.acr & PRFTBE != 0 {
            self.acr |= PRFTBS;
        } else {
            self.acr &= !PRFTBS;
        }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => {
                self.sync_acr_status_bits();
                self.acr
            }
            0x0004 => self.keyr,
            0x0008 => self.optkeyr,
            0x000C => self.sr,
            0x0010 => self.cr,
            0x0014 => self.ar,
            0x001C => self.obr,
            0x0020 => self.wrpr,
            _ => {
                trace!("{} read unknown offset=0x{:04x}", self.name, offset);
                0
            }
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => {
                self.acr = value;
                self.sync_acr_status_bits();
            }
            0x0004 => self.keyr = value,
            0x0008 => self.optkeyr = value,
            0x000C => self.sr = value,
            0x0010 => self.cr = value,
            0x0014 => self.ar = value,
            0x001C => self.obr = value,
            0x0020 => self.wrpr = value,
            _ => {
                trace!("{} write unknown offset=0x{:04x} value=0x{:08x}", self.name, offset, value);
            }
        }
    }
}
