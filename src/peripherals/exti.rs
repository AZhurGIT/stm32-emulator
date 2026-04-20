// SPDX-License-Identifier: GPL-3.0-or-later

use crate::system::System;

use super::Peripheral;

#[derive(Default)]
pub struct Exti {
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl Exti {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "EXTI" {
            Some(Box::new(Self::default()))
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

    fn trigger_line(&mut self, sys: &System, line: u8) {
        let bit = 1u32 << line;
        self.pr |= bit;
        if (self.imr & bit) != 0 {
            if let Some(irq) = Self::irq_for_line(line) {
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
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.imr = value,
            0x04 => self.emr = value,
            0x08 => self.rtsr = value,
            0x0C => self.ftsr = value,
            0x10 => self.trigger_from_swier(sys, value),
            // In STM32, writing 1 clears the pending bit.
            0x14 => self.pr &= !value,
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

    // STM32F1 EXTI base address.
    const EXTI_BASE: u32 = 0x4001_0400;
    // Snapshot trigger masks from the EXTI peripheral model.
    let rtsr = sys.p.read(sys, EXTI_BASE + 0x08, 4);
    let ftsr = sys.p.read(sys, EXTI_BASE + 0x0C, 4);
    let bit = 1u32 << line;
    let trigger = (rising && (rtsr & bit) != 0) || (falling && (ftsr & bit) != 0);
    if trigger {
        // SWIER write delegates to Exti::trigger_line() and raises NVIC if IMR allows it.
        sys.p.write(sys, EXTI_BASE + 0x10, 4, bit as u64);
    }
}
