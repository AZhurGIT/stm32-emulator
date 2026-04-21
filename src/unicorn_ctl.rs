// SPDX-License-Identifier: GPL-3.0-or-later

//! Unicorn `uc_ctl` helpers missing from the official Rust bindings.

use core::ffi::c_void;

use unicorn_engine::{unicorn_const::uc_error, Unicorn};

extern "C" {
    /// Implemented in `c/unicorn_ctl_shim.c` — calls variadic `uc_ctl` with correct C ABI.
    fn stm32_emu_uc_ctl_tb_flush(engine: *mut c_void) -> i32;
}

/// Invalidate all translation blocks (TB cache), like C `uc_ctl_flush_tlb(uc)`.
#[inline]
pub fn tb_flush_all(uc: &Unicorn<()>) -> Result<(), uc_error> {
    let code = unsafe { stm32_emu_uc_ctl_tb_flush(uc.get_handle()) };
    if code == uc_error::OK as i32 {
        Ok(())
    } else {
        // uc_err matches repr(C) uc_error for known codes
        Err(unsafe { std::mem::transmute::<i32, uc_error>(code) })
    }
}
