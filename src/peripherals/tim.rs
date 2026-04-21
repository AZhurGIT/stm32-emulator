// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;
use super::meta::{DeviceMeta, PeripheralMeta};
use super::{nvic, Peripheral};

#[derive(Default)]
pub struct Tim {
    name: String,
    update_irq: Option<i32>,
    reg_cr1: u32,
    reg_cr2: u32,
    reg_smcr: u32,
    reg_dier: u32,
    reg_sr: u32,
    reg_egr: u32,
    reg_cnt: u32,
    reg_psc: u32,
    reg_arr: u32,
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
    pub fn new(name: &str, device_meta: &DeviceMeta, meta: Option<&PeripheralMeta>) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("TIM") {
            let reg = |n: &str, default: u32| meta.and_then(|m| m.offset_of(n)).unwrap_or(default);
            Some(Box::new(Self {
                name: name.to_string(),
                update_irq: device_meta
                    .irq_of(name)
                    .or_else(|| if name == "TIM4" { Some(nvic::irq::TIM4) } else { None }),
                reg_cr1: reg("CR1", 0x0000),
                reg_cr2: reg("CR2", 0x0004),
                reg_smcr: reg("SMCR", 0x0008),
                reg_dier: reg("DIER", 0x000C),
                reg_sr: reg("SR", 0x0010),
                reg_egr: reg("EGR", 0x0014),
                reg_cnt: reg("CNT", 0x0024),
                reg_psc: reg("PSC", 0x0028),
                reg_arr: reg("ARR", 0x002C),
                ..Self::default()
            }))
        } else {
            None
        }
    }

    fn update_irq_config(&self, sys: &System) {
        if self.update_irq != Some(nvic::irq::TIM4) {
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
            o if o == self.reg_cr1 => self.cr1,
            o if o == self.reg_cr2 => self.cr2,
            o if o == self.reg_smcr => self.smcr,
            o if o == self.reg_dier => self.dier,
            o if o == self.reg_sr => {
                if self.update_irq == Some(nvic::irq::TIM4) && sys.p.nvic.borrow().tim4_update_pending {
                    self.sr |= 1; // UIF
                }
                self.sr
            }
            o if o == self.reg_egr => self.egr,
            o if o == self.reg_cnt => self.cnt,
            o if o == self.reg_psc => self.psc,
            o if o == self.reg_arr => self.arr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            o if o == self.reg_cr1 => {
                self.cr1 = value;
                self.update_irq_config(sys);
            }
            o if o == self.reg_cr2 => self.cr2 = value,
            o if o == self.reg_smcr => self.smcr = value,
            o if o == self.reg_dier => {
                self.dier = value;
                self.update_irq_config(sys);
            }
            o if o == self.reg_sr => {
                self.sr = value;
                if self.update_irq == Some(nvic::irq::TIM4) && (value & 1) == 0 {
                    sys.p.nvic.borrow_mut().clear_tim4_update_pending();
                }
            }
            o if o == self.reg_egr => {
                self.egr = value;
                if let Some(irq) = self.update_irq.filter(|_| (value & 1) != 0) {
                    self.sr |= 1;
                    if irq == nvic::irq::TIM4 {
                        sys.p.nvic.borrow_mut().tim4_update_pending = true;
                    }
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
            o if o == self.reg_cnt => self.cnt = value,
            o if o == self.reg_psc => {
                self.psc = value;
                self.update_irq_config(sys);
            }
            o if o == self.reg_arr => {
                self.arr = value;
                self.update_irq_config(sys);
            }
            _ => {
                trace!("{} write unknown offset=0x{:04x} value=0x{:08x}", self.name, offset, value);
            }
        }
    }
}
