// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use svd_parser::svd::{Device as SvdDevice, PeripheralInfo};

#[derive(Clone, Default)]
pub struct PeripheralMeta {
    pub name: String,
    pub base: u32,
    offsets_by_name: HashMap<String, u32>,
}

impl PeripheralMeta {
    pub fn offset_of(&self, register_name: &str) -> Option<u32> {
        self.offsets_by_name.get(register_name).copied()
    }

    pub fn has_register(&self, register_name: &str) -> bool {
        self.offsets_by_name.contains_key(register_name)
    }
}

#[derive(Clone, Default)]
pub struct DeviceMeta {
    peripherals: HashMap<String, PeripheralMeta>,
    interrupts: HashMap<String, i32>,
}

impl DeviceMeta {
    pub fn from_svd(svd_device: &SvdDevice) -> Self {
        let svd_peripherals = svd_device
            .peripherals
            .iter()
            .map(|d| (d.name.to_string(), d))
            .collect::<HashMap<_, _>>();

        let mut peripherals = HashMap::new();
        let mut interrupts = HashMap::new();

        for p in &svd_device.peripherals {
            let source = if let Some(derived_from) = p.derived_from.as_ref() {
                *svd_peripherals
                    .get(derived_from)
                    .as_ref()
                    .unwrap_or_else(|| panic!("Cannot find peripheral {}", derived_from))
            } else {
                p
            };

            let mut offsets_by_name = HashMap::new();
            for r in crate::util::extract_svd_registers(source) {
                offsets_by_name.insert(r.name.to_string(), r.address_offset);
            }

            peripherals.insert(
                p.name.to_string(),
                PeripheralMeta {
                    name: p.name.to_string(),
                    base: p.base_address as u32,
                    offsets_by_name,
                },
            );

            Self::collect_interrupts(&mut interrupts, p);
            if !source.name.eq_ignore_ascii_case(&p.name) {
                Self::collect_interrupts(&mut interrupts, source);
            }
        }

        Self {
            peripherals,
            interrupts,
        }
    }

    fn collect_interrupts(interrupts: &mut HashMap<String, i32>, p: &PeripheralInfo) {
        for intr in &p.interrupt {
            interrupts.insert(intr.name.to_string(), intr.value as i32);
        }
    }

    pub fn peripheral(&self, peripheral_name: &str) -> Option<&PeripheralMeta> {
        self.peripherals.get(peripheral_name)
    }

    pub fn peripheral_base(&self, peripheral_name: &str) -> Option<u32> {
        self.peripheral(peripheral_name).map(|p| p.base)
    }

    pub fn offset_of(&self, peripheral_name: &str, register_name: &str) -> Option<u32> {
        self.peripheral(peripheral_name)
            .and_then(|p| p.offset_of(register_name))
    }

    pub fn has_register(&self, peripheral_name: &str, register_name: &str) -> bool {
        self.peripheral(peripheral_name)
            .map(|p| p.has_register(register_name))
            .unwrap_or(false)
    }

    pub fn irq_of(&self, interrupt_name: &str) -> Option<i32> {
        self.interrupts.get(interrupt_name).copied()
    }

    pub fn interrupt_map(&self) -> &HashMap<String, i32> {
        &self.interrupts
    }
}

#[cfg(test)]
mod tests {
    use super::DeviceMeta;

    #[test]
    fn resolves_offsets_and_interrupts_from_svd() {
        let svd = r#"
<device xmlns:xs="http://www.w3.org/2001/XMLSchema-instance" schemaVersion="1.1" xs:noNamespaceSchemaLocation="CMSIS-SVD_Schema_1_1.xsd">
  <name>TEST</name>
  <version>1.0</version>
  <description>test</description>
  <addressUnitBits>8</addressUnitBits>
  <width>32</width>
  <peripherals>
    <peripheral>
      <name>USART1</name>
      <baseAddress>0x40011000</baseAddress>
      <interrupt>
        <name>USART1</name>
        <value>37</value>
      </interrupt>
      <registers>
        <register>
          <name>SR</name>
          <addressOffset>0x0</addressOffset>
        </register>
        <register>
          <name>DR</name>
          <addressOffset>0x4</addressOffset>
        </register>
      </registers>
    </peripheral>
  </peripherals>
</device>
"#;
        let device = svd_parser::parse(svd).expect("svd parse");
        let meta = DeviceMeta::from_svd(&device);

        assert_eq!(meta.peripheral_base("USART1"), Some(0x4001_1000));
        assert_eq!(meta.offset_of("USART1", "SR"), Some(0x0));
        assert_eq!(meta.offset_of("USART1", "DR"), Some(0x4));
        assert!(meta.has_register("USART1", "SR"));
        assert_eq!(meta.irq_of("USART1"), Some(37));
    }

    #[test]
    fn returns_none_for_missing_entries() {
        let svd = r#"
<device xmlns:xs="http://www.w3.org/2001/XMLSchema-instance" schemaVersion="1.1" xs:noNamespaceSchemaLocation="CMSIS-SVD_Schema_1_1.xsd">
  <name>TEST</name>
  <version>1.0</version>
  <description>test</description>
  <addressUnitBits>8</addressUnitBits>
  <width>32</width>
  <peripherals>
    <peripheral>
      <name>GPIOA</name>
      <baseAddress>0x40010800</baseAddress>
      <registers />
    </peripheral>
  </peripherals>
</device>
"#;
        let device = svd_parser::parse(svd).expect("svd parse");
        let meta = DeviceMeta::from_svd(&device);

        assert_eq!(meta.offset_of("EXTI", "RTSR"), None);
        assert_eq!(meta.irq_of("EXTI0"), None);
        assert!(!meta.has_register("GPIOA", "MODER"));
    }
}
