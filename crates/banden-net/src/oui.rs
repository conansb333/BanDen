//! MAC vendor lookup + multi-tier vendor/device-type resolution.
//!
//! Vendor pipeline:
//!   1. IEEE OUI table (offline, curated) - only meaningful for
//!      globally-unique (burned-in) MACs.
//!   2. Randomized-MAC detection (locally administered bit set: second
//!      hex digit of the first octet is 2, 6, A or E) - the real vendor
//!      is unknowable; display a privacy fallback unless the hostname
//!      reveals the brand.
//!   3. Hostname keyword extraction (Infinix, Redmi, OPPO, HONOR, ...).
//!
//! Device-type fingerprinting (priority order):
//!   1. Router/gateway hostname keywords -> "router"
//!   2. Mobile hostname keywords         -> "smartphone"
//!   3. Computer hostname keywords       -> "desktop"
//!   4. Other host keywords (NAS, printer, TV, SBC)
//!   5. OUI vendor class (virtualization, SBC, printer)
//!   6. None -> the UI shows "unknown", never a guessed "desktop"

use std::collections::HashMap;
use std::sync::OnceLock;

/// (OUI prefix, uppercase hex without separators, vendor name).
const BUILT_IN: &[(&str, &str)] = &[
    // Apple
    ("000393", "Apple"),
    ("ACDE48", "Apple"),
    ("F0DBE2", "Apple"),
    ("A44CC1", "Apple"),
    // Microsoft
    ("0050F2", "Microsoft"),
    ("002248", "Microsoft"),
    ("0025AE", "Microsoft"),
    ("00155D", "Microsoft"),
    // Networking gear
    ("00000C", "Cisco"),
    ("00237E", "Cisco"),
    ("001B0C", "Cisco"),
    ("F866F2", "Cisco"),
    ("000625", "Linksys"),
    ("0023CD", "Linksys"),
    ("001F33", "Netgear"),
    ("9C3DCF", "Netgear"),
    ("14D64D", "TP-Link"),
    ("50C7BF", "TP-Link"),
    ("001E58", "D-Link"),
    ("00055D", "D-Link"),
    ("002401", "Asustek Computer"),
    ("3C970E", "Huawei Device"),
    // PC / server vendors
    ("0017C9", "Intel Corporate"),
    ("3C7C3F", "Intel Corporate"),
    ("A434D9", "Intel Corporate"),
    ("F8B156", "Hewlett Packard"),
    ("3CD92B", "Hewlett Packard"),
    ("D8CB8A", "Dell"),
    ("F8BC12", "Dell"),
    ("E48D8C", "Dell"),
    ("345A60", "Realtek Semiconductor"),
    // Virtualization (these also drive the "virtual machine" type rule)
    ("000C29", "VMware"),
    ("005056", "VMware"),
    ("000D3A", "VMware"),
    ("001C14", "VMware"),
    ("00163E", "Xensource"),
    ("080027", "VirtualBox"),
    ("525400", "QEMU"),
    // Hobby / SBC
    ("B827EB", "Raspberry Pi Foundation"),
    ("DCA632", "Raspberry Pi Trading"),
    ("E45F01", "Raspberry Pi Trading"),
    // Storage / printers / misc
    ("001132", "Synology"),
    ("00046A", "Epson"),
    ("0023AE", "Epson"),
    ("00112F", "Lite-On Technology"),
    ("A4F1C8", "Samsung Electronics"),
    ("002399", "Samsung Electronics"),
    ("001A11", "Google"),
    ("F4F5D8", "Google"),
    ("70B3D5", "IEEE Registration Authority"),
    // Wi-Fi modules common in USB adapters
    ("00E04C", "Realtek Semiconductor"),
];

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        BUILT_IN
            .iter()
            .filter(|(k, _)| k.len() == 6 && k.bytes().all(|b| b.is_ascii_hexdigit()))
            .copied()
            .collect()
    })
}

/// Hex digits of a MAC, uppercase (separators stripped).
fn mac_hex(mac: &str) -> String {
    mac.chars()
        .filter(|c| c.is_ascii_hexdigit())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// True when the MAC is locally administered (randomized / MAC privacy):
/// the second hex digit of the first octet is 2, 6, A or E. The real
/// vendor of such an address cannot be looked up.
pub fn is_randomized_mac(mac: &str) -> bool {
    let hex = mac_hex(mac);
    if hex.len() < 2 {
        return false;
    }
    let first = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    first & 0x02 != 0
}

/// Tier 1: IEEE OUI lookup - only meaningful for non-randomized MACs.
pub fn lookup_vendor(mac: &str) -> Option<&'static str> {
    let clean = mac_hex(mac);
    if clean.len() < 6 {
        return None;
    }
    table().get(&clean[..6]).copied()
}

/// Tier 3: brand extraction from the hostname.
pub fn vendor_from_hostname(hostname: &str) -> Option<&'static str> {
    let h = hostname.to_lowercase();
    let brands: &[(&str, &str)] = &[
        ("infinix", "Infinix Mobility"),
        ("redmi", "Xiaomi"),
        ("xiaomi", "Xiaomi"),
        ("poco", "Xiaomi"),
        ("oppo", "OPPO"),
        ("honor", "HONOR"),
        ("huawei", "Huawei"),
        ("samsung", "Samsung"),
        ("galaxy", "Samsung"),
        ("iphone", "Apple"),
        ("ipad", "Apple"),
        ("macbook", "Apple"),
        ("realme", "Realme"),
        ("vivo", "Vivo"),
        ("pixel", "Google"),
        ("nokia", "Nokia"),
        ("motorola", "Motorola"),
        ("asus", "ASUSTeK"),
        ("acer", "Acer"),
        ("lenovo", "Lenovo"),
        ("thinkpad", "Lenovo"),
    ];
    brands.iter().find(|(k, _)| h.contains(k)).map(|(_, v)| *v)
}

/// Result of the vendor resolution pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorResolution {
    pub vendor: Option<String>,
    pub randomized: bool,
}

/// Multi-tier vendor resolution:
/// randomized MAC -> hostname brand -> OUI table -> privacy fallback.
pub fn resolve_vendor(mac: &str, hostname: Option<&str>) -> VendorResolution {
    let randomized = is_randomized_mac(mac);
    let host_vendor = hostname.and_then(vendor_from_hostname);

    if randomized {
        VendorResolution {
            vendor: Some(
                host_vendor
                    .unwrap_or("Randomized (MAC privacy)")
                    .to_string(),
            ),
            randomized: true,
        }
    } else if let Some(v) = lookup_vendor(mac) {
        VendorResolution {
            vendor: Some(v.to_string()),
            randomized: false,
        }
    } else {
        VendorResolution {
            vendor: host_vendor.map(|v| v.to_string()),
            randomized: false,
        }
    }
}

const ROUTER_KEYWORDS: &[&str] = &[
    "homerouter",
    "gateway",
    "cpe",
    "tplink",
    "tp-link",
    "router",
    "openwrt",
    "fritzbox",
];
const PHONE_KEYWORDS: &[&str] = &[
    "infinix",
    "redmi",
    "xiaomi",
    "poco",
    "oppo",
    "honor",
    "samsung",
    "galaxy",
    "iphone",
    "android",
    "realme",
    "vivo",
    "pixel",
    "nokia",
    "motorola",
    "smartphone",
];
const COMPUTER_KEYWORDS: &[&str] = &[
    "this pc",
    "desktop-",
    "desktop",
    "macbook",
    "thinkpad",
    "laptop",
    "workstation",
    "pc-",
    "notebook",
];

/// Multi-tier device-type fingerprinting. Returns None for genuinely
/// unknown devices - the UI renders that as "unknown", never as a guessed
/// "desktop".
pub fn guess_device_type(mac: &str, hostname: Option<&str>) -> Option<&'static str> {
    let host = hostname.unwrap_or("").to_lowercase();

    // 1. Routers/gateways first.
    if ROUTER_KEYWORDS.iter().any(|k| host.contains(k)) {
        return Some("router");
    }
    // 2. Phones.
    if PHONE_KEYWORDS.iter().any(|k| host.contains(k)) {
        return Some("smartphone");
    }
    // 3. Computers.
    if COMPUTER_KEYWORDS.iter().any(|k| host.contains(k)) {
        return Some("desktop");
    }
    // 4. Other host classes.
    if host.contains("raspberrypi") {
        return Some("single-board computer");
    }
    if host.contains("nas") || host.contains("synology") {
        return Some("nas");
    }
    if host.contains("printer") {
        return Some("printer");
    }
    if host.contains("tv") || host.contains("cast") {
        return Some("media");
    }

    // 5. OUI vendor class - meaningless for randomized MACs.
    if is_randomized_mac(mac) {
        return None;
    }
    match lookup_vendor(mac) {
        Some("VMware" | "VirtualBox" | "QEMU" | "Xensource") => Some("virtual machine"),
        Some("Raspberry Pi Foundation") | Some("Raspberry Pi Trading") => {
            Some("single-board computer")
        }
        Some("Epson") => Some("printer"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_vendors() {
        assert_eq!(
            lookup_vendor("B8:27:EB:12:34:56"),
            Some("Raspberry Pi Foundation")
        );
        assert_eq!(lookup_vendor("00:50:56:A1:B2:C3"), Some("VMware"));
        assert_eq!(lookup_vendor("00:50:F2:11:22:33"), Some("Microsoft"));
        assert_eq!(lookup_vendor("08:00:27:00:00:01"), Some("VirtualBox"));
        assert_eq!(lookup_vendor("52:54:00:12:34:56"), Some("QEMU"));
        assert_eq!(
            lookup_vendor("34:5A:60:11:22:33"),
            Some("Realtek Semiconductor")
        );
    }

    #[test]
    fn unknown_vendor_returns_none() {
        assert_eq!(lookup_vendor("9C:2E:A1:2C:0A:99"), None); // not in table
        assert_eq!(lookup_vendor("garbage"), None);
    }

    #[test]
    fn randomized_detection() {
        // second hex digit of the first octet in {2,6,A,E}
        assert!(is_randomized_mac("EE:6F:54:83:24:52")); // E
        assert!(is_randomized_mac("7A:6A:50:55:DC:F4")); // A
        assert!(is_randomized_mac("56:A6:5B:9E:55:17")); // 6
        assert!(is_randomized_mac("9E:11:22:33:44:55")); // E
        assert!(is_randomized_mac("22:99:FE:E7:89:B1")); // 2 (local admin)
        assert!(!is_randomized_mac("08:00:27:67:ED:3E")); // 8
        assert!(!is_randomized_mac("34:5A:60:C7:D7:B7")); // 4
        assert!(!is_randomized_mac("00:50:56:A1:B2:C3")); // 0
        assert!(!is_randomized_mac("AC:DE:48:00:00:01")); // C
        assert!(!is_randomized_mac("zz:zz:zz")); // no hex digits at all
    }

    #[test]
    fn hostname_vendor_extraction() {
        assert_eq!(
            vendor_from_hostname("Infinix-HOT-60i"),
            Some("Infinix Mobility")
        );
        assert_eq!(vendor_from_hostname("M2006C3MG-Redmi9C"), Some("Xiaomi"));
        assert_eq!(vendor_from_hostname("OPPO-A3x"), Some("OPPO"));
        assert_eq!(vendor_from_hostname("HONOR-X6b"), Some("HONOR"));
        assert_eq!(vendor_from_hostname("galaxy-s24"), Some("Samsung"));
        assert_eq!(vendor_from_hostname("homerouter.cpe"), None);
        assert_eq!(vendor_from_hostname(""), None);
    }

    #[test]
    fn resolve_vendor_pipeline() {
        // Randomized + revealing hostname -> brand.
        let r = resolve_vendor("EE:6F:54:83:24:52", Some("Infinix-HOT-60i"));
        assert!(r.randomized);
        assert_eq!(r.vendor.as_deref(), Some("Infinix Mobility"));

        // Randomized, anonymous hostname -> privacy fallback.
        let r = resolve_vendor("EE:6F:54:83:24:52", None);
        assert!(r.randomized);
        assert_eq!(r.vendor.as_deref(), Some("Randomized (MAC privacy)"));

        // Burned-in MAC -> OUI table wins over hostname.
        let r = resolve_vendor("08:00:27:67:ED:3E", None);
        assert!(!r.randomized);
        assert_eq!(r.vendor.as_deref(), Some("VirtualBox"));

        // Burned-in MAC (bit1 clear), unknown OUI, brand in hostname.
        let r = resolve_vendor("44:D9:E7:2C:0A:99", Some("M2006C3MG-Redmi9C"));
        assert!(!r.randomized);
        assert_eq!(r.vendor.as_deref(), Some("Xiaomi"));

        // Burned-in MAC, unknown OUI, no hostname hints.
        let r = resolve_vendor("44:D9:E7:2C:0A:99", None);
        assert!(!r.randomized);
        assert_eq!(r.vendor, None);
    }

    #[test]
    fn device_type_priority_rules() {
        // Router keywords first.
        assert_eq!(
            guess_device_type("22:99:FE:E7:89:B1", Some("homerouter.cpe")),
            Some("router")
        );
        // Phone keywords (these previously fell through to nothing/"desktop").
        assert_eq!(
            guess_device_type("EE:6F:54:83:24:52", Some("Infinix-HOT-60i")),
            Some("smartphone")
        );
        assert_eq!(
            guess_device_type("9C:2E:A1:2C:0A:99", Some("M2006C3MG-Redmi9C")),
            Some("smartphone")
        );
        assert_eq!(
            guess_device_type("7A:6A:50:55:DC:F4", Some("OPPO-A3x")),
            Some("smartphone")
        );
        assert_eq!(
            guess_device_type("56:A6:5B:9E:55:17", Some("HONOR-X6b")),
            Some("smartphone")
        );
        // Computer keywords.
        assert_eq!(
            guess_device_type("34:5A:60:C7:D7:B7", Some("DESKTOP-0BKJRCL")),
            Some("desktop")
        );
        assert_eq!(
            guess_device_type("34:5A:60:C7:D7:B7", Some("This PC")),
            Some("desktop")
        );
        // OUI class for VMs.
        assert_eq!(
            guess_device_type("08:00:27:67:ED:3E", None),
            Some("virtual machine")
        );
        assert_eq!(
            guess_device_type("00:50:56:AA:BB:CC", None),
            Some("virtual machine")
        );
        // Genuinely unknown -> None (never a guessed "desktop").
        assert_eq!(guess_device_type("9C:2E:A1:2C:0A:99", None), None);
    }

    #[test]
    fn built_in_table_is_clean() {
        for (k, v) in BUILT_IN {
            assert!(
                k.len() == 6 && k.bytes().all(|b| b.is_ascii_hexdigit()),
                "bad key {k}"
            );
            assert!(!v.is_empty());
        }
    }
}
