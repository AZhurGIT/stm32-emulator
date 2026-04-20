// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, rc::Rc};

use anyhow::Result;
use serde::Deserialize;

use crate::{
    framebuffers::sdl_engine::{virtual_input_level, VirtualInputPin},
    framebuffers::{Framebuffer, Framebuffers, RGB888},
    peripherals::exti,
    peripherals::gpio::{GpioPorts, Pin},
};

#[derive(Debug, Deserialize)]
pub struct LcdSSD1306Config {
    pub framebuffer: String,
    pub data_pins: Option<Vec<String>>,
    pub enable_pin: Option<String>,
    pub write_read_pin: Option<String>,
    pub data_command_pin: Option<String>,
    pub chip_select_pin: Option<String>,
    pub reset_pin: Option<String>,
    pub pullup_inputs: Option<Vec<String>>,
    pub led_pin: Option<String>,
    #[serde(default = "default_scale")]
    pub scale: usize,
}

fn default_scale() -> usize {
    1
}

pub struct LcdSSD1306 {
    pub config: LcdSSD1306Config,
    framebuffer: Rc<RefCell<dyn Framebuffer<RGB888>>>,
    name: String,
    trace_enabled: bool,

    data_bus: u8,
    page: u8,
    column: u8,
    col_start: u8,
    col_end: u8,
    page_start: u8,
    page_end: u8,
    horizontal_addressing: bool,

    enable: bool,
    write_read: bool,
    data_command: bool,
    chip_select: bool,
    reset: bool,
    pending_cmd: Option<PendingCommand>,
}

impl LcdSSD1306 {
    pub fn register(config: LcdSSD1306Config, gpio: &mut GpioPorts, framebuffers: &Framebuffers) -> Result<Rc<RefCell<Self>>> {
        let framebuffer = framebuffers.get(&config.framebuffer)?;
        let self_ = Rc::new(RefCell::new(Self {
            config,
            framebuffer,
            name: "LCD-SSD1306".to_string(),
            trace_enabled: true,
            data_bus: 0,
            page: 0,
            column: 0,
            col_start: 0,
            col_end: 127,
            page_start: 0,
            page_end: 7,
            horizontal_addressing: false,
            enable: false,
            write_read: false,
            data_command: false,
            chip_select: true,
            reset: true,
            pending_cmd: None,
        }));

        {
            let pins = self_
                .borrow()
                .config
                .data_pins
                .clone()
                .unwrap_or_else(|| (0..=7).map(|p| format!("PC{}", p)).collect());
            assert!(pins.len() == 8, "data_pins must have exactly 8 pins");
            for (bit, pin) in pins.iter().enumerate() {
                let pin = Pin::from_str(pin);
                let s = self_.clone();
                gpio.add_write_callback(pin, move |_sys, v| {
                    s.borrow_mut().write_data_bit(bit as u8, v);
                });
            }
        }

        {
            let pin = self_
                .borrow()
                .config
                .enable_pin
                .clone()
                .unwrap_or_else(|| "PB7".to_string());
            let pin = Pin::from_str(&pin);
            let s = self_.clone();
            gpio.add_write_callback(pin, move |_sys, v| {
                s.borrow_mut().write_enable(v);
            });
        }

        {
            let pin = self_
                .borrow()
                .config
                .write_read_pin
                .clone()
                .unwrap_or_else(|| "PB6".to_string());
            let pin = Pin::from_str(&pin);
            let s = self_.clone();
            gpio.add_write_callback(pin, move |_sys, v| {
                s.borrow_mut().write_write_read(v);
            });
        }

        {
            let pin = self_
                .borrow()
                .config
                .data_command_pin
                .clone()
                .unwrap_or_else(|| "PB5".to_string());
            let pin = Pin::from_str(&pin);
            let s = self_.clone();
            gpio.add_write_callback(pin, move |_sys, v| {
                s.borrow_mut().write_data_command(v);
            });
        }

        {
            let pin = self_
                .borrow()
                .config
                .chip_select_pin
                .clone()
                .unwrap_or_else(|| "PC11".to_string());
            let pin = Pin::from_str(&pin);
            let s = self_.clone();
            gpio.add_write_callback(pin, move |_sys, v| {
                s.borrow_mut().write_chip_select(v);
            });
        }

        if let Some(pin) = self_.borrow().config.reset_pin.clone() {
            let pin = Pin::from_str(&pin);
            let s = self_.clone();
            gpio.add_write_callback(pin, move |_sys, v| {
                s.borrow_mut().write_reset(v);
            });
        }

        for pin_name in self_.borrow().config.pullup_inputs.clone().unwrap_or_default() {
            let pin = Pin::from_str(&pin_name);
            let maybe_virtual_pin = virtual_input_pin_for_name(&pin_name);
            let maybe_exti_line = virtual_input_exti_line_for_name(&pin_name);
            let mut prev_level: Option<bool> = None;
            gpio.add_read_callback(pin, move |sys| {
                let level = if let Some(vpin) = maybe_virtual_pin {
                    virtual_input_level(vpin)
                } else {
                    true
                };
                if let (Some(line), Some(prev)) = (maybe_exti_line, prev_level) {
                    if prev != level {
                        exti::on_input_transition(sys, line, prev, level);
                    }
                }
                prev_level = Some(level);
                level
            });
        }

        if let Some(pin) = self_.borrow().config.led_pin.clone() {
            let pin = Pin::from_str(&pin);
            gpio.add_write_callback(pin, move |_sys, v| {
                debug!("LCD-SSD1306 LED={}", if v { "on" } else { "off" });
            });
        }

        Ok(self_)
    }

    fn write_data_bit(&mut self, bit: u8, value: bool) {
        let mask = 1u8 << bit;
        if value {
            self.data_bus |= mask;
        } else {
            self.data_bus &= !mask;
        }
    }

    fn write_enable(&mut self, value: bool) {
        // Latch value on falling edge (EN: 1 -> 0)
        if self.enable && !value {
            self.latch();
        }
        self.enable = value;
    }

    fn write_write_read(&mut self, value: bool) {
        self.write_read = value;
    }

    fn write_data_command(&mut self, value: bool) {
        self.data_command = value;
    }

    fn write_chip_select(&mut self, value: bool) {
        self.chip_select = value;
    }

    fn write_reset(&mut self, value: bool) {
        // Active-low reset: when rising from 0->1, consider reset complete.
        let was_low = !self.reset;
        self.reset = value;
        if was_low && value {
            self.clear();
        }
    }

    fn latch(&mut self) {
        // CS is active low in this board wiring.
        if self.chip_select {
            return;
        }

        // WR/RD: false means write cycle.
        if self.write_read {
            return;
        }

        if self.data_command {
            self.write_data(self.data_bus);
        } else {
            self.write_command(self.data_bus);
        }
    }

    fn write_command(&mut self, cmd: u8) {
        if let Some(mut pending) = self.pending_cmd.take() {
            pending.args.push(cmd);
            if pending.args.len() == pending.args_needed {
                self.execute_command(pending.cmd, &pending.args);
            } else {
                self.pending_cmd = Some(pending);
            }
            return;
        }

        let args_needed = command_arg_count(cmd);
        if args_needed == 0 {
            self.execute_command(cmd, &[]);
        } else {
            self.pending_cmd = Some(PendingCommand {
                cmd,
                args_needed,
                args: Vec::with_capacity(args_needed),
            });
        }
    }

    fn write_data(&mut self, byte: u8) {
        if self.trace_enabled {
            trace!("{} data=0x{:02x} page={} col={}", self.name, byte, self.page, self.column);
        }
        self.draw_byte(self.column as usize, self.page as usize, byte);
        if self.column < self.col_end {
            self.column = self.column.wrapping_add(1);
        } else {
            self.column = self.col_start;
            if self.horizontal_addressing {
                if self.page < self.page_end {
                    self.page = self.page.wrapping_add(1);
                } else {
                    self.page = self.page_start;
                }
            }
        }
    }

    fn draw_byte(&mut self, x: usize, page: usize, byte: u8) {
        let (width, height) = {
            let fb = self.framebuffer.borrow();
            let c = fb.get_config();
            (c.width as usize, c.height as usize)
        };

        let scale = self.config.scale.max(1);

        if x * scale >= width {
            return;
        }

        let mut fb = self.framebuffer.borrow_mut();
        let pixels = fb.get_pixels();

        for bit in 0..8usize {
            let y = page * 8 + bit;
            if y * scale >= height {
                break;
            }

            let on = (byte >> bit) & 1 != 0;
            let c = if on { 0x00FF_FFFF } else { 0x0000_0000 };

            for sy in 0..scale {
                let py = y * scale + sy;
                if py >= height {
                    break;
                }
                for sx in 0..scale {
                    let px = x * scale + sx;
                    if px >= width {
                        break;
                    }
                    pixels[px + py * width] = c;
                }
            }
        }
    }

    fn clear(&mut self) {
        self.page = 0;
        self.column = 0;
        self.col_start = 0;
        self.col_end = 127;
        self.page_start = 0;
        self.page_end = 7;
        self.horizontal_addressing = false;
        self.data_bus = 0;
        self.pending_cmd = None;
        let mut fb = self.framebuffer.borrow_mut();
        for p in fb.get_pixels().iter_mut() {
            *p = 0;
        }
    }

    fn execute_command(&mut self, cmd: u8, args: &[u8]) {
        if self.trace_enabled {
            trace!("{} cmd=0x{:02x} args={:02x?}", self.name, cmd, args);
        }

        match (cmd, args) {
            (0xB0..=0xB7, []) => {
                self.page = cmd & 0x07;
            }
            (0x10..=0x1F, []) => {
                self.column = (self.column & 0x0F) | ((cmd & 0x0F) << 4);
            }
            (0x00..=0x0F, []) => {
                self.column = (self.column & 0xF0) | (cmd & 0x0F);
            }
            // Set column range (SSD1306 style): use start column for writes.
            (0x21, [start, _end]) => {
                self.column = *start;
                self.col_start = *start;
                self.col_end = *(_end);
            }
            // Set page range (SSD1306 style): use start page for writes.
            (0x22, [start, _end]) => {
                self.page = *start;
                self.page_start = *start;
                self.page_end = *(_end);
            }
            // Set memory addressing mode: 0 = horizontal.
            (0x20, [mode]) => {
                self.horizontal_addressing = *mode == 0;
            }
            // Start line / display on-off / contrast / mux / offsets etc.
            _ => {}
        }
    }

    pub fn set_name(&mut self, index: usize) {
        self.name = format!("LCD-SSD1306-{}", index);
    }

    pub fn set_trace_enabled(&mut self, enabled: bool) {
        self.trace_enabled = enabled;
    }
}

struct PendingCommand {
    cmd: u8,
    args_needed: usize,
    args: Vec<u8>,
}

fn command_arg_count(cmd: u8) -> usize {
    match cmd {
        0x21 | 0x22 => 2,
        0x20 | 0x81 | 0xA8 | 0xD3 | 0xD5 | 0xD9 | 0xDA | 0xDB | 0xFD => 1,
        _ => 0,
    }
}

fn virtual_input_pin_for_name(name: &str) -> Option<VirtualInputPin> {
    let name = name.to_uppercase();
    match name.as_str() {
        "PA4" => Some(VirtualInputPin::LeftEncoderA),
        "PA5" => Some(VirtualInputPin::LeftEncoderB),
        "PB11" => Some(VirtualInputPin::RightEncoderA),
        "PB13" => Some(VirtualInputPin::RightEncoderB),
        // Firmware mapping: EXTI0 (PA0) -> right, EXTI9_5 line6 (PA6) -> left.
        "PA0" => Some(VirtualInputPin::RightButton),
        "PA6" => Some(VirtualInputPin::LeftButton),
        _ => None,
    }
}

fn virtual_input_exti_line_for_name(name: &str) -> Option<u8> {
    let name = name.to_uppercase();
    match name.as_str() {
        "PA0" => Some(0),
        "PA6" => Some(6),
        "PB14" => Some(14),
        _ => None,
    }
}
