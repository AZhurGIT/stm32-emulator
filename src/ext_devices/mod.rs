// SPDX-License-Identifier: GPL-3.0-or-later

mod spi_flash;
mod usart_probe;
mod display;
mod lcd;
mod touchscreen;
mod lcd_ssd1306;

use spi_flash::{SpiFlashConfig, SpiFlash};
use usart_probe::{UsartProbeConfig, UsartProbe};
use display::{DisplayConfig, Display};
use lcd::{LcdConfig, Lcd};
use touchscreen::{TouchscreenConfig, Touchscreen};
use lcd_ssd1306::{LcdSSD1306Config, LcdSSD1306};

use std::{cell::RefCell, collections::HashSet, rc::Rc};
use serde::Deserialize;
use anyhow::Result;

use crate::{system::System, framebuffers::Framebuffers, peripherals::gpio::GpioPorts};


#[derive(Debug, Deserialize, Default)]
pub struct ExtDevicesConfig {
    /// Optional whitelist for external-device trace logs.
    /// Supported keys: spi_flash, usart_probe, display, lcd, touchscreen, lcd_ssd1306.
    /// If omitted, behavior is unchanged.
    pub trace_devices: Option<Vec<String>>,
    pub spi_flash: Option<Vec<SpiFlashConfig>>,
    pub usart_probe: Option<Vec<UsartProbeConfig>>,
    pub display: Option<Vec<DisplayConfig>>,
    pub lcd: Option<Vec<LcdConfig>>,
    pub touchscreen: Option<Vec<TouchscreenConfig>>,
    pub lcd_ssd1306: Option<Vec<LcdSSD1306Config>>,
}

pub struct ExtDevices {
    pub spi_flashes: Vec<Rc<RefCell<SpiFlash>>>,
    pub usart_probes: Vec<Rc<RefCell<UsartProbe>>>,
    pub displays: Vec<Rc<RefCell<Display>>>,
    pub lcds: Vec<Rc<RefCell<Lcd>>>,
    pub touchscreens: Vec<Rc<RefCell<Touchscreen>>>,
    pub lcd_ssd1306s: Vec<Rc<RefCell<LcdSSD1306>>>,
}

impl ExtDevices {
    pub fn find_serial_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        self.spi_flashes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
        .or_else(||
        self.usart_probes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
       )
        .or_else(||
        self.lcds.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
       )
        .or_else(||
        self.touchscreens.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
       )
    }

    pub fn find_mem_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>> {
        self.displays.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<u32, u32>>>)
    }
}

impl ExtDevicesConfig {
    pub fn into_ext_devices(self, gpio: &mut GpioPorts, framebuffers: &Framebuffers) -> Result<ExtDevices> {
        let filter = self.trace_devices.as_ref().map(|v| {
            v.iter()
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<HashSet<_>>()
        });
        let trace_enabled = |name: &str| {
            filter
                .as_ref()
                .map_or(true, |set| set.contains("*") || set.contains(name))
        };

        let spi_flashes = self.spi_flash.unwrap_or_default().into_iter()
            .map(|config| SpiFlash::new(config).map(RefCell::new).map(Rc::new))
            .collect::<Result<_>>()?;

        let usart_probes = self.usart_probe.unwrap_or_default().into_iter()
            .map(|config| {
                let mut dev = UsartProbe::new(config)?;
                dev.set_trace_enabled(trace_enabled("usart_probe"));
                Ok(Rc::new(RefCell::new(dev)))
            })
            .collect::<Result<_>>()?;

        let displays = self.display.unwrap_or_default().into_iter()
            .map(|config| {
                let mut dev = Display::new(config, framebuffers)?;
                dev.set_trace_enabled(trace_enabled("display"));
                Ok(Rc::new(RefCell::new(dev)))
            })
            .collect::<Result<_>>()?;

        let lcds = self.lcd.unwrap_or_default().into_iter()
            .map(|config| Lcd::new(config, framebuffers).map(RefCell::new).map(Rc::new))
            .collect::<Result<_>>()?;

        let touchscreens = self.touchscreen.unwrap_or_default().into_iter()
            .map(|config| Touchscreen::new(config, gpio, framebuffers).map(RefCell::new).map(Rc::new))
            .collect::<Result<_>>()?;

        let lcd_ssd1306s = self.lcd_ssd1306.unwrap_or_default().into_iter()
            .enumerate()
            .map(|(i, config)| {
                let device = LcdSSD1306::register(config, gpio, framebuffers)?;
                device
                    .borrow_mut()
                    .set_trace_enabled(trace_enabled("lcd_ssd1306"));
                device.borrow_mut().set_name(i);
                Ok(device)
            })
            .collect::<Result<_>>()?;

        Ok(ExtDevices { spi_flashes, usart_probes, displays, lcds, touchscreens, lcd_ssd1306s })
    }
}

///////////////////////////////////////////////////////////////////////////////////////

pub trait ExtDevice<A, T> {
    /// Should returns "{peri_name} {ext_device_name}"
    fn connect_peripheral<'a>(&mut self, peri_name: &str) -> String;
    fn read(&mut self, sys: &System, addr: A) -> T;
    fn write(&mut self, sys: &System, addr: A, v: T);
    fn available(&self) -> usize { 0 }
}
