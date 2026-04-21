// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{system::System, ext_devices::{ExtDevices, ExtDevice}};
use super::meta::PeripheralMeta;
use super::Peripheral;

pub struct Fsmc {
    banks: [Bank; 4],
    register_access: HashMap<u32, (usize, Reg)>,
}

impl Fsmc {
    pub fn new(name: &str, ext_devices: &ExtDevices, meta: Option<&PeripheralMeta>) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("FSMC") {
            let banks = [
                Bank::new(0, ext_devices),
                Bank::new(1, ext_devices),
                Bank::new(2, ext_devices),
                Bank::new(3, ext_devices),
            ];
            Some(Box::new(Self {
                banks,
                register_access: Self::register_access_from_meta(meta),
            }))
        } else {
            None
        }
    }

    fn register_access_from_meta(meta: Option<&PeripheralMeta>) -> HashMap<u32, (usize, Reg)> {
        let mut map = HashMap::new();
        let Some(meta) = meta else {
            return map;
        };

        let mut add_bank = |bank: usize, name: String, reg: Reg| {
            if let Some(offset) = meta.offset_of(name.as_str()) {
                map.insert(offset, (bank, reg));
            }
        };

        for bank in 0..4usize {
            let b = bank + 1;
            add_bank(bank, format!("BCR{}", b), Reg::BCR);
            add_bank(bank, format!("BTR{}", b), Reg::BTR);
            add_bank(bank, format!("BWTR{}", b), Reg::BWTR);
        }
        for bank in 1..4usize {
            let b = bank + 1;
            add_bank(bank, format!("PCR{}", b), Reg::PCR);
            add_bank(bank, format!("SR{}", b), Reg::SR);
            add_bank(bank, format!("PMEM{}", b), Reg::PMEM);
            add_bank(bank, format!("PATT{}", b), Reg::PATT);
            add_bank(bank, format!("ECCR{}", b), Reg::ECCR);
        }
        add_bank(3, "PIO4".to_string(), Reg::PIO);

        map
    }

    fn fallback_register_access(offset: u32) -> (usize, Reg) {
        match offset {
            0x0000 => (0, Reg::BCR),
            0x0004 => (0, Reg::BTR),
            0x0008 => (1, Reg::BCR),
            0x000C => (1, Reg::BTR),
            0x0010 => (2, Reg::BCR),
            0x0014 => (2, Reg::BTR),
            0x0018 => (3, Reg::BCR),
            0x001C => (3, Reg::BTR),
            0x0060 => (1, Reg::PCR),
            0x0064 => (1, Reg::SR),
            0x0068 => (1, Reg::PMEM),
            0x006C => (1, Reg::PATT),
            0x0074 => (1, Reg::ECCR),
            0x0080 => (2, Reg::PCR),
            0x0084 => (2, Reg::SR),
            0x0088 => (2, Reg::PMEM),
            0x008C => (2, Reg::PATT),
            0x0094 => (2, Reg::ECCR),
            0x00A0 => (3, Reg::PCR),
            0x00A4 => (3, Reg::SR),
            0x00A8 => (3, Reg::PMEM),
            0x00AC => (3, Reg::PATT),
            0x00B0 => (3, Reg::PIO),
            0x0104 => (0, Reg::BWTR),
            0x010C => (1, Reg::BWTR),
            0x0114 => (2, Reg::BWTR),
            0x011C => (3, Reg::BWTR),
            _ => (0, Reg::Invalid),
        }
    }

    fn access(&self, offset: u32) -> Access {
        match offset {
            0x0000_0000..=0x0fff_ffff => Access::Data(0, offset),
            0x1000_0000..=0x1fff_ffff => Access::Data(1, offset - 0x1000_0000),
            0x2000_0000..=0x2fff_ffff => Access::Data(2, offset - 0x2000_0000),
            0x3000_0000..=0x3fff_ffff => Access::Data(3, offset - 0x3000_0000),
            0x4000_0000..=0x4fff_ffff => {
                let reg_offset = offset - 0x4000_0000;
                let (bank, reg) = self
                    .register_access
                    .get(&reg_offset)
                    .copied()
                    .unwrap_or_else(|| Self::fallback_register_access(reg_offset));
                Access::Register(bank, reg)
            }
            _ => unreachable!()
        }
    }
}

impl Peripheral for Fsmc {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match self.access(offset) {
            Access::Data(bank, offset) => self.banks[bank].read_data(sys, offset),
            Access::Register(bank, reg) => self.banks[bank].read_reg(sys, reg),
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match self.access(offset) {
            Access::Data(bank, offset) => self.banks[bank].write_data(sys, offset, value),
            Access::Register(bank, reg) => self.banks[bank].write_reg(sys, reg, value),
        }
    }
}

pub trait FsmcDevice {
    fn name(&self, fsmc_bank_name: &str) -> String;
    fn read_data(&mut self, bank: &mut Bank, offset: u32) -> u32;
    fn write_data(&mut self, bank: &mut Bank, offset: u32, value: u32);
}

pub struct Bank {
    pub name: String,
    ext_device: Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>>,
}

impl Bank {
    pub fn new(bank: usize, ext_devices: &ExtDevices) -> Self {
        let name = format!("FSMC.BANK{}", bank+1);

        let ext_device = ext_devices.find_mem_device(&name);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);

        Self { name, ext_device }
    }

    fn read_data(&mut self, sys: &System, offset: u32) -> u32 {
        let v = self.ext_device.as_ref().map(|d|
            d.borrow_mut().read(sys, offset)
        ).unwrap_or_default();

        trace!("{} data read at offset=0x{:08x} value=0x{:08x}", self.name, offset, v);

        v
    }

    fn write_data(&mut self, sys: &System, offset: u32, value: u32) {
        self.ext_device.as_ref().map(|d|
            d.borrow_mut().write(sys, offset, value)
        );

        trace!("{} data write at offset=0x{:08x} value=0x{:08x}", self.name, offset, value);
    }

    fn read_reg(&mut self, _sys: &System, reg: Reg) -> u32 {
        trace!("{} read reg={:?}", self.name, reg);
        0
    }

    fn write_reg(&mut self, _sys: &System, reg: Reg, _value: u32) {
        trace!("{} write reg={:?}", self.name, reg);
    }
}

enum Access {
    Data(usize, u32),
    Register(usize, Reg),
}

#[derive(Debug, Clone, Copy)]
enum Reg {
    BCR,
    BTR,
    PMEM,
    PATT,
    ECCR,
    PCR,
    SR,
    BWTR,
    PIO,
    Invalid,
}
