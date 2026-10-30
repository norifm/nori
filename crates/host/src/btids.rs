//! Bluetooth devices in the form plugins see them (BlueZ's and Android's): an address as
//! `58:18:62:25:85:CB`, a UUID as its 128 bits, lowercase, a dual-mode device once.

use nori_plugins::BtDevice;

/// The Bluetooth base UUID, which a 16 or 32 bit UUID fills the first four bytes of.
const BASE: [u8; 16] = [0, 0, 0, 0, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0x80, 0x5f, 0x9b, 0x34, 0xfb];

/// An address with any separator (macOS's `58-18-62-25-85-cb`), as plugins see it.
pub fn address(text: &str) -> String {
    text.split([':', '-']).map(str::to_uppercase).collect::<Vec<_>>().join(":")
}

/// The 16 bytes of a 2, 4 or 16 byte UUID, as sent (most significant first).
pub fn long(bytes: &[u8]) -> Option<[u8; 16]> {
    let mut out = BASE;
    match bytes.len() {
        2 => out[2..4].copy_from_slice(bytes),
        4 => out[..4].copy_from_slice(bytes),
        16 => out.copy_from_slice(bytes),
        _ => return None,
    }
    Some(out)
}

/// A 2, 4 or 16 byte UUID as text.
pub fn text(bytes: &[u8]) -> Option<String> {
    let b = long(bytes)?;
    let hex = |r: std::ops::Range<usize>| b[r].iter().map(|x| format!("{x:02x}")).collect::<String>();
    Some(format!("{}-{}-{}-{}-{}", hex(0..4), hex(4..6), hex(6..8), hex(8..10), hex(10..16)))
}

/// The bytes of a UUID written in full (`0000180f-0000-1000-8000-00805f9b34fb`) or short (`180F`).
pub fn bytes(text: &str) -> Option<[u8; 16]> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if !matches!(hex.len(), 4 | 8 | 32) || !hex.is_ascii() {
        return None;
    }
    let raw: Option<Vec<u8>> = (0..hex.len()).step_by(2).map(|at| u8::from_str_radix(&hex[at..at + 2], 16).ok()).collect();
    long(&raw?)
}

/// Whether `device` is a CoreBluetooth identifier (`6F1C2E1A-...`, 36 characters), not an address.
pub fn is_peripheral_id(device: &str) -> bool {
    device.len() == 36 && device.matches('-').count() == 4
}

/// The classic devices, each with the services of the LE peripheral of its name, then the peripherals
/// only on LE.
pub fn merged(mut classic: Vec<BtDevice>, le: Vec<BtDevice>) -> Vec<BtDevice> {
    for d in le {
        match classic.iter_mut().find(|c| !c.name.is_empty() && c.name == d.name) {
            Some(c) => {
                for s in d.services {
                    if !c.services.contains(&s) {
                        c.services.push(s);
                    }
                }
            }
            None => classic.push(d),
        }
    }
    classic
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_reads_as_plugins_see_it() {
        assert_eq!(address("58-18-62-25-85-cb"), "58:18:62:25:85:CB");
        assert_eq!(address("58:18:62:25:85:CB"), "58:18:62:25:85:CB");
    }

    #[test]
    fn a_uuid_of_any_length_reads_in_full() {
        let battery = "0000180f-0000-1000-8000-00805f9b34fb";
        assert_eq!(text(&[0x18, 0x0f]).as_deref(), Some(battery));
        assert_eq!(text(&[0, 0, 0x18, 0x0f]).as_deref(), Some(battery));
        assert_eq!(text(&bytes(battery).unwrap()).as_deref(), Some(battery));
        assert_eq!(bytes("180F"), bytes(battery));
        assert_eq!(text(&bytes("0000FE2C-0000-1000-8000-00805F9B34FB").unwrap()).as_deref(), Some("0000fe2c-0000-1000-8000-00805f9b34fb"));
        assert_eq!(text(&[1, 2, 3]), None);
        assert_eq!(bytes("not a uuid"), None);
        assert_eq!(bytes("éé"), None);
    }

    #[test]
    fn a_peripheral_id_is_told_from_an_address() {
        assert!(is_peripheral_id("6F1C2E1A-0000-4000-8000-000000000001"));
        assert!(!is_peripheral_id("58:18:62:25:85:CB"));
    }

    #[test]
    fn a_dual_mode_device_is_listed_once_with_both_services() {
        let device = |id: &str, name: &str, services: &[&str]| BtDevice { id: id.into(), name: name.into(), services: services.iter().map(|s| s.to_string()).collect(), vendor: None, product: None };
        let classic = vec![device("58:18:62:25:85:CB", "WH-1000XM6", &["spp", "battery"])];
        let le = vec![device("6F1C2E1A-0000-4000-8000-000000000001", "WH-1000XM6", &["battery", "gatt"]), device("6F1C2E1A-0000-4000-8000-000000000002", "Ring", &["battery"])];
        let all = merged(classic, le);
        assert_eq!(all.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), ["58:18:62:25:85:CB", "6F1C2E1A-0000-4000-8000-000000000002"]);
        assert_eq!(all[0].services, ["spp", "battery", "gatt"]);
    }
}
