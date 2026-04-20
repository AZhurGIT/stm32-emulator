// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::atomic::Ordering;

use unicorn_engine::{RegisterARM, Unicorn};

use crate::system::System;
use super::Peripheral;

#[derive(Default)]
pub struct Nvic {
    pub systick_period: Option<u32>,
    pub last_systick_trigger: u64,
    pub tim4_period: Option<u32>,
    pub last_tim4_trigger: u64,
    pub tim4_update_pending: bool,

    /// Cortex-M3 does not implement CONTROL.FPCA (bit 2); it is RES0 in the
    /// architecture but QEMU may expose a non-zero value. Treating that as
    /// "floating-point context active" builds the wrong EXC_RETURN and pops
    /// an extended exception frame, corrupting MSP vs our basic `push_regs`.
    pub force_basic_exc_stack: bool,

    /// Shadow of NVIC ISER0..2 (IRQ 0..95 enable). Writes were previously ignored,
    /// so firmware NVIC_EnableIRQ() had no effect and we could deliver TIM4 before
    /// the vector was armed — confusing init and stack usage.
    iser: [u32; 3],

    // 128 different interrupts. Good enough for now
    pending: u128,
    in_interrupt: bool,
}

const IRQ_OFFSET: i32 = 16;

/// EPSR [15:10] IT state and [26:25] ICI — cleared in the **stacked** xPSR on
/// hardware exception entry (ARMv7-M). If we inject an IRQ while the guest
/// sits inside an IT block, keeping these bits breaks condexec after return.
const XPSR_IT_ICI_MASK: u32 = 0x0600_FC00;

fn read_u16_le(uc: &Unicorn<()>, addr: u32) -> Option<u16> {
    let mut b = [0u8; 2];
    uc.mem_read(addr as u64, &mut b).ok()?;
    Some(u16::from_le_bytes(b))
}

/// First halfword of a 32-bit Thumb-2 instruction (ARM DDI 0406 A5.1).
fn thumb32_first_halfword(h: u16) -> bool {
    let op = (h >> 11) & 0x1f;
    matches!(op, 0x1d | 0x1e | 0x1f)
}

fn thumb_instruction_size(h0: u16) -> u32 {
    if thumb32_first_halfword(h0) {
        4
    } else {
        2
    }
}

/// Number of conditional instructions in an IT block from the IT instruction's low nibble.
/// See ARMv7-M ARM IT encoding (`mask` field): uses `4 - ctz(mask | 0x10)`.
fn it_block_conditional_instruction_count(mask_lo: u8) -> Option<usize> {
    let m = mask_lo & 0xf;
    if m == 0 {
        return None;
    }
    let x = u32::from(m | 0x10);
    Some((4 - x.trailing_zeros()) as usize)
}

/// True if `pc` points into one of the conditional instructions following an `IT` at `it_addr`.
fn pc_in_it_block_after(uc: &Unicorn<()>, it_addr: u32, pc: u32) -> bool {
    let h = match read_u16_le(uc, it_addr) {
        Some(v) => v,
        None => return false,
    };
    if (h >> 8) != 0xbf || h == 0xbf00 {
        return false;
    }
    let n = match it_block_conditional_instruction_count((h & 0xff) as u8) {
        Some(n) => n,
        None => return false,
    };
    let mut p = it_addr.wrapping_add(2);
    if pc < p {
        return false;
    }
    for _ in 0..n {
        let hw = match read_u16_le(uc, p) {
            Some(v) => v,
            None => return false,
        };
        let len = thumb_instruction_size(hw);
        let end = p.wrapping_add(len);
        if pc >= p && pc < end {
            return true;
        }
        p = end;
    }
    false
}

/// If `pc` lies inside a Thumb IT block (decoded from memory), do not inject an IRQ — Unicorn
/// often omits ITSTATE on the boundary after `ite` / `it` (see e.g. `ite eq` at 0x08000520).
fn thumb_pc_inside_it_block_from_memory(uc: &Unicorn<()>, pc: u32) -> bool {
    // Prefer the IT closest to `pc` (largest `it_addr`): scan small deltas first.
    let mut best = false;
    for delta in (2..=24).step_by(2) {
        let it_addr = pc.wrapping_sub(delta);
        if pc_in_it_block_after(uc, it_addr, pc) {
            best = true;
            break;
        }
    }
    best
}

fn guest_in_it_block(uc: &Unicorn<()>) -> bool {
    // QEMU exposes Thumb IT execution state separately from xPSR (RegisterARM::ITSTATE).
    // Delivery from a code hook while non-zero can leave the CPU running IT epilogue in the
    // old TB after we set PC → `ldr pc,[sp]` loads 0 / corrupts flow (PREFETCH_ABORT to 0x0).
    if let Ok(v) = uc.reg_read(RegisterARM::ITSTATE) {
        if (v as u32) & 0xff != 0 {
            return true;
        }
    }
    let xpsr = uc.reg_read(RegisterARM::XPSR).unwrap_or(0) as u32;
    if (xpsr & XPSR_IT_ICI_MASK) != 0 {
        return true;
    }
    // Unicorn may clear ITSTATE/xPSR IT bits between IT and the conditional insns (e.g. at
    // 0x08000522 after `ite eq` at 0x08000520); decode IT blocks from memory as a fallback.
    let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32;
    thumb_pc_inside_it_block_from_memory(uc, pc)
}

pub mod irq {
    pub const PENDSV: i32 = -2;
    pub const SYSTICK: i32 = -1;
    pub const TIM4: i32 = 30;
}

// This is all poorly implemented. If this is not making much sense, it might be
// best to re-implement everything correctly. Right now, I'm just trying to get
// the saturn firmware to work just well enough.

impl Nvic {
    pub fn set_intr_pending(&mut self, irq: i32) {
        trace!("Set irq pending irq={}", irq);
        let bit = IRQ_OFFSET + irq;
        assert!(bit > 0);
        self.pending |= 1 << (IRQ_OFFSET + irq);
    }

    /// Takes the lowest pending IRQ whose NVIC enable bit is set (ISER); skips
    /// disabled lines but leaves them pending (STM32 IRQn maps to ISER bits).
    fn pop_next_deliverable_pending(&mut self) -> Option<i32> {
        let mut mask = self.pending;
        while mask != 0 {
            let bit = mask.trailing_zeros();
            let irq = (bit as i32) - IRQ_OFFSET;
            if self.irq_line_enabled(irq) {
                self.pending &= !(1u128 << bit);
                return Some(irq);
            }
            mask &= !(1u128 << bit);
        }
        None
    }

    fn irq_line_enabled(&self, irq: i32) -> bool {
        if irq < 0 {
            // SysTick / PendSV are not enabled via NVIC ISER; gated elsewhere for now.
            return true;
        }
        let n = irq as u32;
        let idx = (n / 32) as usize;
        if idx >= self.iser.len() {
            return false;
        }
        (self.iser[idx] & (1 << (n % 32))) != 0
    }

    pub fn maybe_set_systick_intr_pending(&mut self) {
        if let Some(systick_period) = self.systick_period {
            let n = crate::emulator::NUM_INSTRUCTIONS.load(Ordering::Relaxed);
            let delta_num_instructions = n - self.last_systick_trigger;
            if delta_num_instructions > (systick_period as u64) {
                self.last_systick_trigger = n;
                self.set_intr_pending(irq::SYSTICK);
            }
        }
    }

    pub fn maybe_set_tim4_intr_pending(&mut self) {
        if let Some(tim4_period) = self.tim4_period {
            let n = crate::emulator::NUM_INSTRUCTIONS.load(Ordering::Relaxed);
            let delta_num_instructions = n - self.last_tim4_trigger;
            if delta_num_instructions > (tim4_period as u64) {
                self.last_tim4_trigger = n;
                self.tim4_update_pending = true;
                self.set_intr_pending(irq::TIM4);
            }
        }
    }

    pub fn clear_tim4_update_pending(&mut self) {
        self.tim4_update_pending = false;
    }

   fn are_interrupts_disabled(sys: &System) -> bool {
        let primask = sys.uc.borrow().reg_read(RegisterARM::PRIMASK).unwrap();
        primask != 0
    }

    /// Returns `true` if a synthetic IRQ was taken (`PC` set to the vector).
    /// The caller should end the current `emu_start` (e.g. `emu_stop()`) so
    /// Unicorn leaves the partially-translated TB: otherwise `reg_write(PC)`
    /// from a code hook can be ignored inside an IT block (see QEMU
    /// `no_exit_request` / `UC_HOOK_FLAG_NO_STOP`).
    pub fn run_pending_interrupts(&mut self, sys: &System, vector_table_addr: u32) -> bool {
        self.maybe_set_systick_intr_pending();
        self.maybe_set_tim4_intr_pending();

        if Self::are_interrupts_disabled(sys) || self.in_interrupt {
            return false;
        }

        if let Some(irq) = self.pop_next_deliverable_pending() {
            if guest_in_it_block(&sys.uc.borrow()) {
                // Retry on a later instruction boundary — do not consume pending.
                let bit = (IRQ_OFFSET + irq) as u32;
                self.pending |= 1u128 << bit;
                return false;
            }
            self.run_interrupt(sys, vector_table_addr, irq);
            return true;
        }
        false
    }

    fn read_vector_addr(sys: &System, vector_table_addr: u32, irq: i32) -> u32 {
        // 4 because of ptr size
        let vaddr = vector_table_addr + 4*(IRQ_OFFSET + irq) as u32;

        let mut vector = [0,0,0,0];
        sys.uc.borrow().mem_read(vaddr as u64, &mut vector).unwrap();
        u32::from_le_bytes(vector)
    }

    // SPSEL, bit[1], 0 means we use MSP, 1 means we use PSP.
    // FPCA, bit[2], if the processor includes the FP extension.

    fn run_interrupt(&mut self, sys: &System, vector_table_addr: u32, irq: i32) {
        let vector = Self::read_vector_addr(sys, vector_table_addr, irq);

        let mut uc = sys.uc.borrow_mut();

        // SPSEL, bit[1], 0 means we use MSP, 1 means we use PSP.
        // FPCA, bit[2], if the processor includes the FP extension.
        let control_reg = uc.reg_read(RegisterARM::CONTROL).unwrap();
        let spsel = control_reg & (1 << 1) != 0;
        let fpca = !self.force_basic_exc_stack && (control_reg & (1 << 2) != 0);

        trace!("Running interrupt irq={} spsel={} fpca={} vector={:#08x}",
            irq, spsel, fpca, vector);

        Self::push_regs(&mut uc, spsel, fpca);

        // Match hardware: EPSR.IT/ICI cleared for execution in the handler.
        let xpsr = uc.reg_read(RegisterARM::XPSR).unwrap() as u32;
        uc.reg_write(RegisterARM::XPSR, (xpsr & !XPSR_IT_ICI_MASK).into())
            .unwrap();
        // Exception entry clears the Thumb IT state machine (not only xPSR bits).
        let _ = uc.reg_write(RegisterARM::ITSTATE, 0);

        // LR meaning:
        //   EXC_RETURN    Return to      Return stack Frame type
        //   0xFFFF_FFE1   Handler mode   Main         Extended
        //   0xFFFF_FFE9   Thread mode    Main         Extended
        //   0xFFFF_FFED   Thread mode    Process      Extended
        //   0xFFFF_FFF1   Handler mode   Main         Basic
        //   0xFFFF_FFF9   Thread mode    Main         Basic
        //   0xFFFF_FFFD   Thread mode    Process      Basic

        // Right now, we don't supposed nested interrupts.
        let mut lr: u32 = 0xFFFF_FFE9;
        if spsel { lr |= 0b0000_0100; }
        if !fpca { lr |= 0b0001_0000; } // Yes, no fpca means the bit is set
        uc.reg_write(RegisterARM::LR, lr.into()).unwrap();

        uc.reg_write(RegisterARM::IPSR, irq as u64).unwrap();
        uc.reg_write(RegisterARM::PC, vector as u64).unwrap();

        self.in_interrupt = true;
    }

    pub fn return_from_interrupt(&mut self, sys: &System) {
        let mut uc = sys.uc.borrow_mut();

        let lr = uc.reg_read(RegisterARM::LR).unwrap();
        if lr & 0xFFFF_FF00 == 0xFFFF_FF00 {
            let spsel = lr & 0b0000_0100 != 0;
            // EXC_RETURN bit 4 (FTYPE): 0 => extended frame, 1 => basic. M3 has no FP
            // extended frame; if LR is wrong, never pop 17 extra words off MSP.
            let mut extended_frame = lr & 0b0001_0000 == 0;
            if self.force_basic_exc_stack {
                extended_frame = false;
            }

            Self::pop_regs(&mut uc, spsel, extended_frame);

            trace!("Return from interrupt spsel={} extended_frame={} pc=0x{:08x}",
                spsel, extended_frame, uc.reg_read(RegisterARM::PC).unwrap());

            // SPSEL, bit[1], 0 means we use MSP, 1 means we use PSP.
            // FPCA, bit[2], if the processor includes the FP extension.
            let mut control_reg = 0;
            if spsel { control_reg |= 1 << 1; }
            if extended_frame { control_reg |= 2 << 1; }
            uc.reg_write(RegisterARM::CONTROL, control_reg).unwrap();
        } else {
            let control_reg = uc.reg_read(RegisterARM::CONTROL).unwrap();
            let spsel = control_reg & (1 << 1) != 0;
            let fpca = !self.force_basic_exc_stack && (control_reg & (1 << 2) != 0);
            Self::pop_regs(&mut uc, spsel, fpca);

            trace!("Return from interrupt spsel={} fpca={} pc=0x{:08x} -- LR was not right",
                spsel, fpca, uc.reg_read(RegisterARM::PC).unwrap());
        }

        self.in_interrupt = false;
    }

    const CONTEXT_REGS_EXTENDED: [RegisterARM; 17] = [
        RegisterARM::FPSCR,
        RegisterARM::S15,
        RegisterARM::S14,
        RegisterARM::S13,
        RegisterARM::S12,
        RegisterARM::S11,
        RegisterARM::S10,
        RegisterARM::S9,
        RegisterARM::S8,
        RegisterARM::S7,
        RegisterARM::S6,
        RegisterARM::S5,
        RegisterARM::S4,
        RegisterARM::S3,
        RegisterARM::S2,
        RegisterARM::S1,
        RegisterARM::S0,
    ];

    const CONTEXT_REGS: [RegisterARM; 8] = [
        RegisterARM::XPSR,
        RegisterARM::PC,
        RegisterARM::LR,
        RegisterARM::R12,
        RegisterARM::R3,
        RegisterARM::R2,
        RegisterARM::R1,
        RegisterARM::R0,
    ];

    fn push_regs(uc: &mut Unicorn<()>, spsel: bool, fpca: bool) {
        let sp_reg = if spsel { RegisterARM::PSP } else { RegisterARM::MSP };
        let mut sp = uc.reg_read(sp_reg).unwrap();

        let mut push_reg = |reg| {
            let mut v = uc.reg_read(reg).unwrap() as u32;
            if reg == RegisterARM::XPSR {
                v &= !XPSR_IT_ICI_MASK;
            }
            //trace!("push sp=0x{:08x} {:5?}=0x{:08x}", sp, reg, v);
            sp -= 4;
            uc.mem_write(sp, &v.to_le_bytes()).expect("Invalid SP pointer during interrupt");
        };

        if fpca {
            for reg in Self::CONTEXT_REGS_EXTENDED {
                push_reg(reg);
            }
        }
        for reg in Self::CONTEXT_REGS {
            push_reg(reg);
        }
        uc.reg_write(RegisterARM::SP, sp).unwrap();
    }

    fn pop_regs(uc: &mut Unicorn<()>, spsel: bool, fpca: bool) {
        let sp_reg = if spsel { RegisterARM::PSP } else { RegisterARM::MSP };
        let mut sp = uc.reg_read(sp_reg).unwrap();

        let mut pop_reg = |reg| {
            let mut v = [0,0,0,0];
            uc.mem_read(sp, &mut v).expect("Invalid SP pointer during interrupt return");
            let v = u32::from_le_bytes(v);
            //trace!("pop sp=0x{:08x} {:5?}=0x{:08x}", sp, reg, v);
            sp += 4;
            uc.reg_write(reg, v as u64).unwrap();
        };

        for reg in Self::CONTEXT_REGS.iter().rev() {
            pop_reg(*reg);
        }
        if fpca {
            for reg in Self::CONTEXT_REGS_EXTENDED.iter().rev() {
                pop_reg(*reg);
            }
        }
        uc.reg_write(RegisterARM::SP, sp).unwrap();
    }
}

impl Peripheral for Nvic {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x100..=0x108 | 0x180..=0x188 => {
                let i = ((offset & 0x0f) >> 2) as usize;
                self.iser.get(i).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x100..=0x108 => {
                let i = ((offset - 0x100) / 4) as usize;
                if i < 3 {
                    self.iser[i] |= value;
                }
            }
            0x180..=0x188 => {
                let i = ((offset - 0x180) / 4) as usize;
                if i < 3 {
                    self.iser[i] &= !value;
                }
            }
            _ => {}
        }
    }
}

/// The next part is glue. Maybe we could have a better architecture.

pub struct NvicWrapper;

impl NvicWrapper {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "NVIC" {
            Some(Box::new(Self))
        } else {
            None
        }
    }
}

impl Peripheral for NvicWrapper {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        sys.p.nvic.borrow_mut().read(sys, offset)
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        sys.p.nvic.borrow_mut().write(sys, offset, value)
    }
}


/*
0xE000E100 B  REGISTER ISER0 (rw): Interrupt Set-Enable Register
0xE000E104 B  REGISTER ISER1 (rw): Interrupt Set-Enable Register
0xE000E108 B  REGISTER ISER2 (rw): Interrupt Set-Enable Register

0xE000E180 B  REGISTER ICER0 (rw): Interrupt Clear-Enable Register
0xE000E184 B  REGISTER ICER1 (rw): Interrupt Clear-Enable Register
0xE000E188 B  REGISTER ICER2 (rw): Interrupt Clear-Enable Register

0xE000E200 B  REGISTER ISPR0 (rw): Interrupt Set-Pending Register
0xE000E204 B  REGISTER ISPR1 (rw): Interrupt Set-Pending Register
0xE000E208 B  REGISTER ISPR2 (rw): Interrupt Set-Pending Register

0xE000E280 B  REGISTER ICPR0 (rw): Interrupt Clear-Pending Register
0xE000E284 B  REGISTER ICPR1 (rw): Interrupt Clear-Pending Register
0xE000E288 B  REGISTER ICPR2 (rw): Interrupt Clear-Pending Register

0xE000E300 B  REGISTER IABR0 (ro): Interrupt Active Bit Register
0xE000E304 B  REGISTER IABR1 (ro): Interrupt Active Bit Register
0xE000E308 B  REGISTER IABR2 (ro): Interrupt Active Bit Register

0xE000E400 B  REGISTER IPR0 (rw): Interrupt Priority Register
0xE000E404 B  REGISTER IPR1 (rw): Interrupt Priority Register
0xE000E408 B  REGISTER IPR2 (rw): Interrupt Priority Register
0xE000E40C B  REGISTER IPR3 (rw): Interrupt Priority Register
0xE000E410 B  REGISTER IPR4 (rw): Interrupt Priority Register
0xE000E414 B  REGISTER IPR5 (rw): Interrupt Priority Register
0xE000E418 B  REGISTER IPR6 (rw): Interrupt Priority Register
0xE000E41C B  REGISTER IPR7 (rw): Interrupt Priority Register
0xE000E420 B  REGISTER IPR8 (rw): Interrupt Priority Register
0xE000E424 B  REGISTER IPR9 (rw): Interrupt Priority Register
0xE000E428 B  REGISTER IPR10 (rw): Interrupt Priority Register
0xE000E42C B  REGISTER IPR11 (rw): Interrupt Priority Register
0xE000E430 B  REGISTER IPR12 (rw): Interrupt Priority Register
0xE000E434 B  REGISTER IPR13 (rw): Interrupt Priority Register
0xE000E438 B  REGISTER IPR14 (rw): Interrupt Priority Register
0xE000E43C B  REGISTER IPR15 (rw): Interrupt Priority Register
0xE000E440 B  REGISTER IPR16 (rw): Interrupt Priority Register
0xE000E444 B  REGISTER IPR17 (rw): Interrupt Priority Register
0xE000E448 B  REGISTER IPR18 (rw): Interrupt Priority Register
0xE000E44C B  REGISTER IPR19 (rw): Interrupt Priority Register
*/
