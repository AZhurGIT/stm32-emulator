// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;
use super::{nvic, Peripheral};

#[derive(Default)]
pub struct Tim {
    name: String,
    is_tim4: bool,
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    egr: u32,
    cnt: u32,
    psc: u32,
    arr: u32,
}

impl Tim {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("TIM") {
            Some(Box::new(Self {
                name: name.to_string(),
                is_tim4: name == "TIM4",
                ..Self::default()
            }))
        } else {
            None
        }
    }

    fn update_irq_config(&self, sys: &System) {
        if !self.is_tim4 {
            return;
        }

        let enabled = (self.cr1 & 1) != 0 && (self.dier & 1) != 0;
        let period = if enabled {
            let p = (self.psc & 0xFFFF).saturating_add(1);
            let a = (self.arr & 0xFFFF).saturating_add(1);
            Some(p.saturating_mul(a).max(1))
        } else {
            None
        };
        sys.p.nvic.borrow_mut().tim4_period = period;
    }
}

impl Peripheral for Tim {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => self.cr1,
            0x0004 => self.cr2,
            0x0008 => self.smcr,
            0x000C => self.dier,
            0x0010 => {
                if self.is_tim4 && sys.p.nvic.borrow().tim4_update_pending {
                    self.sr |= 1; // UIF
                }
                self.sr
            }
            0x0014 => self.egr,
            0x0024 => self.cnt,
            0x0028 => self.psc,
            0x002C => self.arr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => {
                self.cr1 = value;
                self.update_irq_config(sys);
            }
            0x0004 => self.cr2 = value,
            0x0008 => self.smcr = value,
            0x000C => {
                self.dier = value;
                self.update_irq_config(sys);
            }
            0x0010 => {
                self.sr = value;
                if self.is_tim4 && (value & 1) == 0 {
                    sys.p.nvic.borrow_mut().clear_tim4_update_pending();
                }
            }
            0x0014 => {
                self.egr = value;
                if self.is_tim4 && (value & 1) != 0 {
                    self.sr |= 1;
                    sys.p.nvic.borrow_mut().tim4_update_pending = true;
                    sys.p.nvic.borrow_mut().set_intr_pending(nvic::irq::TIM4);
                }
            }
            0x0024 => self.cnt = value,
            0x0028 => {
                self.psc = value;
                self.update_irq_config(sys);
            }
            0x002C => {
                self.arr = value;
                self.update_irq_config(sys);
            }
            _ => {
                trace!("{} write unknown offset=0x{:04x} value=0x{:08x}", self.name, offset, value);
            }
        }
    }
}
