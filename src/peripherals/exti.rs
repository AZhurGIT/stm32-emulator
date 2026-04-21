// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;

use super::meta::{DeviceMeta, PeripheralMeta};
use super::Peripheral;

#[derive(Default)]
pub struct Exti {
    reg_imr: u32,
    reg_emr: u32,
    reg_rtsr: u32,
    reg_ftsr: u32,
    reg_swier: u32,
    reg_pr: u32,
    irq_line_0: Option<i32>,
    irq_line_1: Option<i32>,
    irq_line_2: Option<i32>,
    irq_line_3: Option<i32>,
    irq_line_4: Option<i32>,
    irq_line_9_5: Option<i32>,
    irq_line_15_10: Option<i32>,
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl Exti {
    pub fn new(name: &str, device_meta: &DeviceMeta, peripheral_meta: Option<&PeripheralMeta>) -> Option<Box<dyn Peripheral>> {
        if name == "EXTI" {
            let mut exti = Self::default();
            exti.reg_imr = peripheral_meta.and_then(|m| m.offset_of("IMR")).unwrap_or(0x00);
            exti.reg_emr = peripheral_meta.and_then(|m| m.offset_of("EMR")).unwrap_or(0x04);
            exti.reg_rtsr = peripheral_meta.and_then(|m| m.offset_of("RTSR")).unwrap_or(0x08);
            exti.reg_ftsr = peripheral_meta.and_then(|m| m.offset_of("FTSR")).unwrap_or(0x0C);
            exti.reg_swier = peripheral_meta.and_then(|m| m.offset_of("SWIER")).unwrap_or(0x10);
            exti.reg_pr = peripheral_meta.and_then(|m| m.offset_of("PR")).unwrap_or(0x14);
            exti.irq_line_0 = device_meta.irq_of("EXTI0");
            exti.irq_line_1 = device_meta.irq_of("EXTI1");
            exti.irq_line_2 = device_meta.irq_of("EXTI2");
            exti.irq_line_3 = device_meta.irq_of("EXTI3");
            exti.irq_line_4 = device_meta.irq_of("EXTI4");
            exti.irq_line_9_5 = device_meta.irq_of("EXTI9_5");
            exti.irq_line_15_10 = device_meta.irq_of("EXTI15_10");
            Some(Box::new(exti))
        } else {
            None
        }
    }

    fn irq_for_line(line: u8) -> Option<i32> {
        match line {
            0 => Some(6),
            1 => Some(7),
            2 => Some(8),
            3 => Some(9),
            4 => Some(10),
            5..=9 => Some(23),
            10..=15 => Some(40),
            _ => None,
        }
    }

    fn irq_for_line_runtime(&self, line: u8) -> Option<i32> {
        match line {
            0 => self.irq_line_0.or_else(|| Self::irq_for_line(0)),
            1 => self.irq_line_1.or_else(|| Self::irq_for_line(1)),
            2 => self.irq_line_2.or_else(|| Self::irq_for_line(2)),
            3 => self.irq_line_3.or_else(|| Self::irq_for_line(3)),
            4 => self.irq_line_4.or_else(|| Self::irq_for_line(4)),
            5..=9 => self.irq_line_9_5.or_else(|| Self::irq_for_line(5)),
            10..=15 => self.irq_line_15_10.or_else(|| Self::irq_for_line(10)),
            _ => None,
        }
    }

    fn trigger_line(&mut self, sys: &System, line: u8) {
        let bit = 1u32 << line;
        self.pr |= bit;
        if (self.imr & bit) != 0 {
            if let Some(irq) = self.irq_for_line_runtime(line) {
                sys.p.nvic.borrow_mut().set_intr_pending(irq);
            }
        }
    }

    fn trigger_from_swier(&mut self, sys: &System, value: u32) {
        self.swier |= value;
        for line in 0..=31u8 {
            let bit = 1u32 << line;
            if (value & bit) != 0 {
                self.trigger_line(sys, line);
            }
        }
    }
}

impl Peripheral for Exti {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            o if o == self.reg_imr => self.imr,
            o if o == self.reg_emr => self.emr,
            o if o == self.reg_rtsr => self.rtsr,
            o if o == self.reg_ftsr => self.ftsr,
            o if o == self.reg_swier => self.swier,
            o if o == self.reg_pr => self.pr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            o if o == self.reg_imr => self.imr = value,
            o if o == self.reg_emr => self.emr = value,
            o if o == self.reg_rtsr => self.rtsr = value,
            o if o == self.reg_ftsr => self.ftsr = value,
            o if o == self.reg_swier => self.trigger_from_swier(sys, value),
            // In STM32, writing 1 clears the pending bit.
            o if o == self.reg_pr => self.pr &= !value,
            _ => {}
        }
    }
}

/// Called by GPIO-input emulation to request EXTI evaluation for a line.
pub fn on_input_transition(sys: &System, line: u8, prev: bool, curr: bool) {
    let rising = !prev && curr;
    let falling = prev && !curr;
    if !rising && !falling {
        return;
    }

    let exti_base = sys.p.peripheral_base("EXTI").unwrap_or(0x4001_0400);
    let rtsr_offset = sys.p.offset_of("EXTI", "RTSR").unwrap_or(0x08);
    let ftsr_offset = sys.p.offset_of("EXTI", "FTSR").unwrap_or(0x0C);
    let swier_offset = sys.p.offset_of("EXTI", "SWIER").unwrap_or(0x10);
    // Snapshot trigger masks from the EXTI peripheral model.
    let rtsr = sys.p.read(sys, exti_base + rtsr_offset, 4);
    let ftsr = sys.p.read(sys, exti_base + ftsr_offset, 4);
    let bit = 1u32 << line;
    let trigger = (rising && (rtsr & bit) != 0) || (falling && (ftsr & bit) != 0);
    if trigger {
        // SWIER write delegates to Exti::trigger_line() and raises NVIC if IMR allows it.
        sys.p.write(sys, exti_base + swier_offset, 4, bit as u64);
    }
}
