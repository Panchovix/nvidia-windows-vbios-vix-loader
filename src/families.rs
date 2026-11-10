//! Explicit PCI architecture inventory from NVIDIA's 615.71.09 product table (families.py, loader part).
//!
//! An architecture gate, not evidence of table support: unknown IDs fail closed. The editor-only entries of
//! families.py (EDITOR_ONLY) are not part of the loader and stay unknown here. No hardware is accessed.

pub const SOURCE: &str = "https://raw.githubusercontent.com/NVIDIA/open-gpu-kernel-modules/615.71.09/README.md";

/// GA100 uses a different GSP firmware; its receiver capacity has not been audited.
pub const GA100_DEVICE_IDS: [u16; 10] = [
    0x20B0, 0x20B2, 0x20B3, 0x20B5, 0x20B7, 0x20BD, 0x20F1, 0x20F3, 0x20F5, 0x20F6,
];

const TURING: &str = "turing";
const AMPERE: &str = "ampere";
const ADA: &str = "ada";
const BLACKWELL: &str = "blackwell-gb20x";
const EXCLUDED: &str = "excluded";

// Turing (TU102 0x1E0x-0x1E7x, TU104 0x1E8x-0x1EFx, TU106 0x1F0x-0x1F7x, TU117 0x1F8x-0x1FFx, TU116 0x218x-0x21Dx)
// is experimental: one RTX 2080 Ti has run an edited board power limit; nothing else is tested on Turing hardware.
/// families.DEVICES of loader 0.4.x, sorted by PCI device ID: (device, family, product). Generated from
/// families.py.
#[rustfmt::skip]
const DEVICES: [(u16, &str, &str); 291] = [
    (0x1E02, TURING, "NVIDIA TITAN RTX"),
    (0x1E04, TURING, "NVIDIA GeForce RTX 2080 Ti"),
    (0x1E07, TURING, "NVIDIA GeForce RTX 2080 Ti"),
    (0x1E09, TURING, "NVIDIA CMP 50HX"),
    (0x1E30, TURING, "Quadro RTX 6000"),
    (0x1E36, TURING, "Quadro RTX 6000"),
    (0x1E78, TURING, "Quadro RTX 8000"),
    (0x1E81, TURING, "NVIDIA GeForce RTX 2080 SUPER"),
    (0x1E82, TURING, "NVIDIA GeForce RTX 2080"),
    (0x1E84, TURING, "NVIDIA GeForce RTX 2070 SUPER"),
    (0x1E87, TURING, "NVIDIA GeForce RTX 2080"),
    (0x1E89, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1E90, TURING, "NVIDIA GeForce RTX 2080"),
    (0x1E91, TURING, "NVIDIA GeForce RTX 2070 Super"),
    (0x1E93, TURING, "NVIDIA GeForce RTX 2080 Super"),
    (0x1EB0, TURING, "Quadro RTX 5000"),
    (0x1EB1, TURING, "Quadro RTX 4000"),
    (0x1EB5, TURING, "Quadro RTX 5000"),
    (0x1EB6, TURING, "Quadro RTX 4000"),
    (0x1EB8, TURING, "Tesla T4"),
    (0x1EC2, TURING, "NVIDIA GeForce RTX 2070 SUPER"),
    (0x1EC7, TURING, "NVIDIA GeForce RTX 2070 SUPER"),
    (0x1ED0, TURING, "NVIDIA GeForce RTX 2080"),
    (0x1ED1, TURING, "NVIDIA GeForce RTX 2070 Super"),
    (0x1ED3, TURING, "NVIDIA GeForce RTX 2080 Super"),
    (0x1EF5, TURING, "Quadro RTX 5000"),
    (0x1F02, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F03, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F06, TURING, "NVIDIA GeForce RTX 2060 SUPER"),
    (0x1F07, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F08, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F0A, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F0B, TURING, "NVIDIA CMP 40HX"),
    (0x1F10, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F11, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F12, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F14, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F15, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F36, TURING, "Quadro RTX 3000"),
    (0x1F42, TURING, "NVIDIA GeForce RTX 2060 SUPER"),
    (0x1F47, TURING, "NVIDIA GeForce RTX 2060 SUPER"),
    (0x1F50, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F51, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F54, TURING, "NVIDIA GeForce RTX 2070"),
    (0x1F55, TURING, "NVIDIA GeForce RTX 2060"),
    (0x1F76, TURING, "Quadro RTX 3000"),
    (0x1F82, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F83, TURING, "NVIDIA GeForce GTX 1630"),
    (0x1F91, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F95, TURING, "NVIDIA GeForce GTX 1650 Ti"),
    (0x1F96, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F97, TURING, "NVIDIA GeForce MX450"),
    (0x1F98, TURING, "NVIDIA GeForce MX450"),
    (0x1F99, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F9C, TURING, "NVIDIA GeForce MX450"),
    (0x1F9D, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1F9F, TURING, "NVIDIA GeForce MX550"),
    (0x1FA0, TURING, "NVIDIA GeForce MX550"),
    (0x1FB0, TURING, "NVIDIA T1000"),
    (0x1FB1, TURING, "NVIDIA T600"),
    (0x1FB2, TURING, "NVIDIA T400"),
    (0x1FB6, TURING, "NVIDIA T600 Laptop GPU"),
    (0x1FB7, TURING, "NVIDIA T550 Laptop GPU"),
    (0x1FB8, TURING, "Quadro T2000"),
    (0x1FB9, TURING, "Quadro T1000"),
    (0x1FBA, TURING, "NVIDIA T600 Laptop GPU"),
    (0x1FBB, TURING, "NVIDIA T500"),
    (0x1FBC, TURING, "NVIDIA T1200 Laptop GPU"),
    (0x1FDD, TURING, "NVIDIA GeForce GTX 1650"),
    (0x1FF0, TURING, "NVIDIA T1000 8GB"),
    (0x1FF2, TURING, "NVIDIA T400 4GB"),
    (0x1FF9, TURING, "Quadro T1000"),
    (0x20B0, AMPERE, "NVIDIA A100-SXM4-40GB"),
    (0x20B2, AMPERE, "NVIDIA A100-SXM4-80GB"),
    (0x20B3, AMPERE, "NVIDIA A100-SXM-64GB"),
    (0x20B5, AMPERE, "NVIDIA A100 80GB PCIe"),
    (0x20B7, AMPERE, "NVIDIA A30"),
    (0x20BD, AMPERE, "NVIDIA A800-SXM4-40GB"),
    (0x20F1, AMPERE, "NVIDIA A100-PCIE-40GB"),
    (0x20F3, AMPERE, "NVIDIA A800-SXM4-80GB"),
    (0x20F5, AMPERE, "NVIDIA A800 80GB PCIe"),
    (0x20F6, AMPERE, "NVIDIA A800 40GB Active"),
    (0x2182, TURING, "NVIDIA GeForce GTX 1660 Ti"),
    (0x2184, TURING, "NVIDIA GeForce GTX 1660"),
    (0x2187, TURING, "NVIDIA GeForce GTX 1650 SUPER"),
    (0x2188, TURING, "NVIDIA GeForce GTX 1650"),
    (0x2189, TURING, "NVIDIA CMP 30HX"),
    (0x2191, TURING, "NVIDIA GeForce GTX 1660 Ti"),
    (0x2192, TURING, "NVIDIA GeForce GTX 1650 Ti"),
    (0x21C4, TURING, "NVIDIA GeForce GTX 1660 SUPER"),
    (0x21D1, TURING, "NVIDIA GeForce GTX 1660 Ti"),
    (0x2203, AMPERE, "NVIDIA GeForce RTX 3090 Ti"),
    (0x2204, AMPERE, "NVIDIA GeForce RTX 3090"),
    (0x2206, AMPERE, "NVIDIA GeForce RTX 3080"),
    (0x2207, AMPERE, "NVIDIA GeForce RTX 3070 Ti"),
    (0x2208, AMPERE, "NVIDIA GeForce RTX 3080 Ti"),
    (0x220A, AMPERE, "NVIDIA GeForce RTX 3080"),
    (0x220D, AMPERE, "NVIDIA CMP 90HX"),
    (0x2216, AMPERE, "NVIDIA GeForce RTX 3080"),
    (0x2230, AMPERE, "NVIDIA RTX A6000"),
    (0x2231, AMPERE, "NVIDIA RTX A5000"),
    (0x2232, AMPERE, "NVIDIA RTX A4500"),
    (0x2233, AMPERE, "NVIDIA RTX A5500"),
    (0x2235, AMPERE, "NVIDIA A40"),
    (0x2236, AMPERE, "NVIDIA A10"),
    (0x2237, AMPERE, "NVIDIA A10G"),
    (0x2238, AMPERE, "NVIDIA A10M"),
    (0x230E, EXCLUDED, "NVIDIA H20 NVL16"),
    (0x2321, EXCLUDED, "NVIDIA H100 NVL"),
    (0x2322, EXCLUDED, "NVIDIA H800 PCIe"),
    (0x2324, EXCLUDED, "NVIDIA H800"),
    (0x2329, EXCLUDED, "NVIDIA H20"),
    (0x232C, EXCLUDED, "NVIDIA H20-3e"),
    (0x2330, EXCLUDED, "NVIDIA H100 80GB HBM3"),
    (0x2331, EXCLUDED, "NVIDIA H100 PCIe"),
    (0x2335, EXCLUDED, "NVIDIA H200"),
    (0x2339, EXCLUDED, "NVIDIA H100"),
    (0x233A, EXCLUDED, "NVIDIA H800 NVL"),
    (0x233B, EXCLUDED, "NVIDIA H200 NVL"),
    (0x2342, EXCLUDED, "NVIDIA GH200 120GB"),
    (0x2348, EXCLUDED, "NVIDIA GH200 144G HBM3e"),
    (0x2414, AMPERE, "NVIDIA GeForce RTX 3060 Ti"),
    (0x2420, AMPERE, "NVIDIA GeForce RTX 3080 Ti Laptop GPU"),
    (0x2438, AMPERE, "NVIDIA RTX A5500 Laptop GPU"),
    (0x2460, AMPERE, "NVIDIA GeForce RTX 3080 Ti Laptop GPU"),
    (0x2482, AMPERE, "NVIDIA GeForce RTX 3070 Ti"),
    (0x2484, AMPERE, "NVIDIA GeForce RTX 3070"),
    (0x2486, AMPERE, "NVIDIA GeForce RTX 3060 Ti"),
    (0x2487, AMPERE, "NVIDIA GeForce RTX 3060"),
    (0x2488, AMPERE, "NVIDIA GeForce RTX 3070"),
    (0x2489, AMPERE, "NVIDIA GeForce RTX 3060 Ti"),
    (0x248A, AMPERE, "NVIDIA CMP 70HX"),
    (0x249C, AMPERE, "NVIDIA GeForce RTX 3080 Laptop GPU"),
    (0x249D, AMPERE, "NVIDIA GeForce RTX 3070 Laptop GPU"),
    (0x24A0, AMPERE, "NVIDIA GeForce RTX 3070 Ti Laptop GPU"),
    (0x24B0, AMPERE, "NVIDIA RTX A4000"),
    (0x24B1, AMPERE, "NVIDIA RTX A4000H"),
    (0x24B6, AMPERE, "NVIDIA RTX A5000 Laptop GPU"),
    (0x24B7, AMPERE, "NVIDIA RTX A4000 Laptop GPU"),
    (0x24B8, AMPERE, "NVIDIA RTX A3000 Laptop GPU"),
    (0x24B9, AMPERE, "NVIDIA RTX A3000 12GB Laptop GPU"),
    (0x24BA, AMPERE, "NVIDIA RTX A4500 Laptop GPU"),
    (0x24BB, AMPERE, "NVIDIA RTX A3000 12GB Laptop GPU"),
    (0x24C7, AMPERE, "NVIDIA GeForce RTX 3060"),
    (0x24C9, AMPERE, "NVIDIA GeForce RTX 3060 Ti"),
    (0x24DC, AMPERE, "NVIDIA GeForce RTX 3080 Laptop GPU"),
    (0x24DD, AMPERE, "NVIDIA GeForce RTX 3070 Laptop GPU"),
    (0x24E0, AMPERE, "NVIDIA GeForce RTX 3070 Ti Laptop GPU"),
    (0x24FA, AMPERE, "NVIDIA RTX A4500 Embedded GPU"),
    (0x2503, AMPERE, "NVIDIA GeForce RTX 3060"),
    (0x2504, AMPERE, "NVIDIA GeForce RTX 3060"),
    (0x2507, AMPERE, "NVIDIA GeForce RTX 3050"),
    (0x2508, AMPERE, "NVIDIA GeForce RTX 3050 OEM"),
    (0x2520, AMPERE, "NVIDIA GeForce RTX 3060 Laptop GPU"),
    (0x2521, AMPERE, "NVIDIA GeForce RTX 3060 Laptop GPU"),
    (0x2523, AMPERE, "NVIDIA GeForce RTX 3050 Ti Laptop GPU"),
    (0x2531, AMPERE, "NVIDIA RTX A2000"),
    (0x2544, AMPERE, "NVIDIA GeForce RTX 3060"),
    (0x2560, AMPERE, "NVIDIA GeForce RTX 3060 Laptop GPU"),
    (0x2563, AMPERE, "NVIDIA GeForce RTX 3050 Ti Laptop GPU"),
    (0x2571, AMPERE, "NVIDIA RTX A2000 12GB"),
    (0x2582, AMPERE, "NVIDIA GeForce RTX 3050"),
    (0x2584, AMPERE, "NVIDIA GeForce RTX 3050"),
    (0x25A0, AMPERE, "NVIDIA GeForce RTX 3050 Ti Laptop GPU"),
    (0x25A2, AMPERE, "NVIDIA GeForce RTX 3050 Laptop GPU"),
    (0x25A5, AMPERE, "NVIDIA GeForce RTX 3050 Laptop GPU"),
    (0x25A6, AMPERE, "NVIDIA GeForce MX570"),
    (0x25A7, AMPERE, "NVIDIA GeForce RTX 2050"),
    (0x25A9, AMPERE, "NVIDIA GeForce RTX 2050"),
    (0x25AA, AMPERE, "NVIDIA GeForce MX570 A"),
    (0x25AB, AMPERE, "NVIDIA GeForce RTX 3050 4GB Laptop GPU"),
    (0x25AC, AMPERE, "NVIDIA GeForce RTX 3050 6GB Laptop GPU"),
    (0x25AD, AMPERE, "NVIDIA GeForce RTX 2050"),
    (0x25B0, AMPERE, "NVIDIA RTX A1000"),
    (0x25B2, AMPERE, "NVIDIA RTX A400"),
    (0x25B6, AMPERE, "NVIDIA A16"),
    (0x25B8, AMPERE, "NVIDIA RTX A2000 Laptop GPU"),
    (0x25B9, AMPERE, "NVIDIA RTX A1000 Laptop GPU"),
    (0x25BA, AMPERE, "NVIDIA RTX A2000 8GB Laptop GPU"),
    (0x25BB, AMPERE, "NVIDIA RTX A500 Laptop GPU"),
    (0x25BC, AMPERE, "NVIDIA RTX A1000 6GB Laptop GPU"),
    (0x25BD, AMPERE, "NVIDIA RTX A500 Laptop GPU"),
    (0x25E0, AMPERE, "NVIDIA GeForce RTX 3050 Ti Laptop GPU"),
    (0x25E2, AMPERE, "NVIDIA GeForce RTX 3050 Laptop GPU"),
    (0x25E5, AMPERE, "NVIDIA GeForce RTX 3050 Laptop GPU"),
    (0x25EC, AMPERE, "NVIDIA GeForce RTX 3050 6GB Laptop GPU"),
    (0x25ED, AMPERE, "NVIDIA GeForce RTX 2050"),
    (0x25F9, AMPERE, "NVIDIA RTX A1000 Embedded GPU"),
    (0x25FA, AMPERE, "NVIDIA RTX A2000 Embedded GPU"),
    (0x25FB, AMPERE, "NVIDIA RTX A500 Embedded GPU"),
    (0x2684, ADA, "NVIDIA GeForce RTX 4090"),
    (0x2685, ADA, "NVIDIA GeForce RTX 4090 D"),
    (0x2689, ADA, "NVIDIA GeForce RTX 4070 Ti SUPER"),
    (0x26B1, ADA, "NVIDIA RTX 6000 Ada Generation"),
    (0x26B2, ADA, "NVIDIA RTX 5000 Ada Generation"),
    (0x26B3, ADA, "NVIDIA RTX 5880 Ada Generation"),
    (0x26B5, ADA, "NVIDIA L40"),
    (0x26B9, ADA, "NVIDIA L40S"),
    (0x26BA, ADA, "NVIDIA L20"),
    (0x2702, ADA, "NVIDIA GeForce RTX 4080 SUPER"),
    (0x2704, ADA, "NVIDIA GeForce RTX 4080"),
    (0x2705, ADA, "NVIDIA GeForce RTX 4070 Ti SUPER"),
    (0x2709, ADA, "NVIDIA GeForce RTX 4070"),
    (0x2717, ADA, "NVIDIA GeForce RTX 4090 Laptop GPU"),
    (0x2730, ADA, "NVIDIA RTX 5000 Ada Generation Laptop GPU"),
    (0x2757, ADA, "NVIDIA GeForce RTX 4090 Laptop GPU"),
    (0x2770, ADA, "NVIDIA RTX 5000 Ada Generation Embedded GPU"),
    (0x2782, ADA, "NVIDIA GeForce RTX 4070 Ti"),
    (0x2783, ADA, "NVIDIA GeForce RTX 4070 SUPER"),
    (0x2786, ADA, "NVIDIA GeForce RTX 4070"),
    (0x2788, ADA, "NVIDIA GeForce RTX 4060 Ti"),
    (0x27A0, ADA, "NVIDIA GeForce RTX 4080 Laptop GPU"),
    (0x27B0, ADA, "NVIDIA RTX 4000 SFF Ada Generation"),
    (0x27B1, ADA, "NVIDIA RTX 4500 Ada Generation"),
    (0x27B2, ADA, "NVIDIA RTX 4000 Ada Generation"),
    (0x27B6, ADA, "NVIDIA L2"),
    (0x27B8, ADA, "NVIDIA L4"),
    (0x27BA, ADA, "NVIDIA RTX 4000 Ada Generation Laptop GPU"),
    (0x27BB, ADA, "NVIDIA RTX 3500 Ada Generation Laptop GPU"),
    (0x27E0, ADA, "NVIDIA GeForce RTX 4080 Laptop GPU"),
    (0x27FB, ADA, "NVIDIA RTX 3500 Ada Generation Embedded GPU"),
    (0x2803, ADA, "NVIDIA GeForce RTX 4060 Ti"),
    (0x2805, ADA, "NVIDIA GeForce RTX 4060 Ti"),
    (0x2808, ADA, "NVIDIA GeForce RTX 4060"),
    (0x2820, ADA, "NVIDIA GeForce RTX 4070 Laptop GPU"),
    (0x2822, ADA, "NVIDIA GeForce RTX 3050 A Laptop GPU"),
    (0x2838, ADA, "NVIDIA RTX 3000 Ada Generation Laptop GPU"),
    (0x2860, ADA, "NVIDIA GeForce RTX 4070 Laptop GPU"),
    (0x2882, ADA, "NVIDIA GeForce RTX 4060"),
    (0x28A0, ADA, "NVIDIA GeForce RTX 4060 Laptop GPU"),
    (0x28A1, ADA, "NVIDIA GeForce RTX 4050 Laptop GPU"),
    (0x28A3, ADA, "NVIDIA GeForce RTX 3050 A Laptop GPU"),
    (0x28B0, ADA, "NVIDIA RTX 2000 Ada Generation"),
    (0x28B8, ADA, "NVIDIA RTX 2000 Ada Generation Laptop GPU"),
    (0x28B9, ADA, "NVIDIA RTX 1000 Ada Generation Laptop GPU"),
    (0x28BA, ADA, "NVIDIA RTX 500 Ada Generation Laptop GPU"),
    (0x28BB, ADA, "NVIDIA RTX 500 Ada Generation Laptop GPU"),
    (0x28E0, ADA, "NVIDIA GeForce RTX 4060 Laptop GPU"),
    (0x28E1, ADA, "NVIDIA GeForce RTX 4050 Laptop GPU"),
    (0x28E3, ADA, "NVIDIA GeForce RTX 3050 A Laptop GPU"),
    (0x28F8, ADA, "NVIDIA RTX 2000 Ada Generation Embedded GPU"),
    (0x2901, EXCLUDED, "NVIDIA B200"),
    (0x2909, EXCLUDED, "NVIDIA B200"),
    (0x2941, EXCLUDED, "NVIDIA GB200"),
    (0x29BB, EXCLUDED, "NVIDIA DRIVE P2021"),
    (0x2B85, BLACKWELL, "NVIDIA GeForce RTX 5090"),
    (0x2B87, BLACKWELL, "NVIDIA GeForce RTX 5090 D"),
    (0x2B8C, BLACKWELL, "NVIDIA GeForce RTX 5090 D v2"),
    (0x2BB1, BLACKWELL, "NVIDIA RTX PRO 6000 Blackwell Workstation Edition"),
    (0x2BB3, BLACKWELL, "NVIDIA RTX PRO 5000 Blackwell"),
    (0x2BB4, BLACKWELL, "NVIDIA RTX PRO 6000 Blackwell Max-Q Workstation Edition"),
    (0x2BB5, BLACKWELL, "NVIDIA RTX PRO 6000 Blackwell Server Edition"),
    (0x2BB9, BLACKWELL, "NVIDIA RTX 6000D"),
    (0x2C02, BLACKWELL, "NVIDIA GeForce RTX 5080"),
    (0x2C05, BLACKWELL, "NVIDIA GeForce RTX 5070 Ti"),
    (0x2C09, BLACKWELL, "NVIDIA GeForce RTX 5070"),
    (0x2C18, BLACKWELL, "NVIDIA GeForce RTX 5090 Laptop GPU"),
    (0x2C19, BLACKWELL, "NVIDIA GeForce RTX 5080 Laptop GPU"),
    (0x2C31, BLACKWELL, "NVIDIA RTX PRO 4500 Blackwell"),
    (0x2C33, BLACKWELL, "NVIDIA RTX PRO 4000 Blackwell SFF Edition"),
    (0x2C34, BLACKWELL, "NVIDIA RTX PRO 4000 Blackwell"),
    (0x2C38, BLACKWELL, "NVIDIA RTX PRO 5000 Blackwell Generation Laptop GPU"),
    (0x2C39, BLACKWELL, "NVIDIA RTX PRO 4000 Blackwell Generation Laptop GPU"),
    (0x2C3A, BLACKWELL, "NVIDIA RTX PRO 4500 Blackwell Server Edition"),
    (0x2C58, BLACKWELL, "NVIDIA GeForce RTX 5090 Laptop GPU"),
    (0x2C59, BLACKWELL, "NVIDIA GeForce RTX 5080 Laptop GPU"),
    (0x2C77, BLACKWELL, "NVIDIA RTX PRO 5000 Blackwell Embedded GPU"),
    (0x2C79, BLACKWELL, "NVIDIA RTX PRO 4000 Blackwell Embedded GPU"),
    (0x2D04, BLACKWELL, "NVIDIA GeForce RTX 5060 Ti"),
    (0x2D05, BLACKWELL, "NVIDIA GeForce RTX 5060"),
    (0x2D18, BLACKWELL, "NVIDIA GeForce RTX 5070 Laptop GPU"),
    (0x2D19, BLACKWELL, "NVIDIA GeForce RTX 5060 Laptop GPU"),
    (0x2D30, BLACKWELL, "NVIDIA RTX PRO 2000 Blackwell"),
    (0x2D39, BLACKWELL, "NVIDIA RTX PRO 2000 Blackwell Generation Laptop GPU"),
    (0x2D58, BLACKWELL, "NVIDIA GeForce RTX 5070 Laptop GPU"),
    (0x2D59, BLACKWELL, "NVIDIA GeForce RTX 5060 Laptop GPU"),
    (0x2D79, BLACKWELL, "NVIDIA RTX PRO 2000 Blackwell Embedded GPU"),
    (0x2D83, BLACKWELL, "NVIDIA GeForce RTX 5050"),
    (0x2D98, BLACKWELL, "NVIDIA GeForce RTX 5050 Laptop GPU"),
    (0x2DB8, BLACKWELL, "NVIDIA RTX PRO 1000 Blackwell Generation Laptop GPU"),
    (0x2DB9, BLACKWELL, "NVIDIA RTX PRO 500 Blackwell Generation Laptop GPU"),
    (0x2DD8, BLACKWELL, "NVIDIA GeForce RTX 5050 Laptop GPU"),
    (0x2DF9, BLACKWELL, "NVIDIA RTX PRO 500 Blackwell Embedded GPU"),
    (0x2F04, BLACKWELL, "NVIDIA GeForce RTX 5070"),
    (0x2F06, BLACKWELL, "NVIDIA GeForce RTX 5060"),
    (0x2F18, BLACKWELL, "NVIDIA GeForce RTX 5070 Ti Laptop GPU"),
    (0x2F38, BLACKWELL, "NVIDIA RTX PRO 3000 Blackwell Generation Laptop GPU"),
    (0x2F58, BLACKWELL, "NVIDIA GeForce RTX 5070 Ti Laptop GPU"),
    (0x3182, EXCLUDED, "NVIDIA B300 SXM6 AC"),
    (0x31C2, EXCLUDED, "NVIDIA GB300"),
    (0x31C3, EXCLUDED, "NVIDIA GB300"),
];

fn entry(device: u16) -> Option<&'static (u16, &'static str, &'static str)> {
    DEVICES.binary_search_by_key(&device, |e| e.0).ok().map(|i| &DEVICES[i])
}

/// "turing", "ampere", "ada", "blackwell-gb20x", "excluded" or "unknown" (families.architecture(device)['family']).
pub fn family(device: u16) -> &'static str {
    entry(device).map_or("unknown", |e| e.1)
}

/// The product name of the table, or "unknown PCI device".
pub fn product(device: u16) -> &'static str {
    entry(device).map_or("unknown PCI device", |e| e.2)
}

pub fn is_ga100(device: u16) -> bool {
    GA100_DEVICE_IDS.contains(&device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_without_duplicates() {
        assert!(DEVICES.windows(2).all(|w| w[0].0 < w[1].0));
        assert_eq!(DEVICES.len(), 291);
    }

    #[test]
    fn lookups() {
        assert_eq!(
            (family(0x2C09), product(0x2C09)),
            ("blackwell-gb20x", "NVIDIA GeForce RTX 5070")
        );
        assert_eq!((family(0x2684), product(0x2684)), ("ada", "NVIDIA GeForce RTX 4090"));
        assert_eq!((family(0x2531), product(0x2531)), ("ampere", "NVIDIA RTX A2000"));
        assert_eq!(
            (family(0x1F06), product(0x1F06)),
            ("turing", "NVIDIA GeForce RTX 2060 SUPER")
        );
        assert_eq!((family(0x2330), product(0x2330)), ("excluded", "NVIDIA H100 80GB HBM3"));
        // EDITOR_ONLY (10de:2501) is unknown to the loader.
        assert_eq!((family(0x2501), product(0x2501)), ("unknown", "unknown PCI device"));
        assert_eq!(family(0x0000), "unknown");
        assert_eq!(family(0xFFFF), "unknown");
        for device in GA100_DEVICE_IDS {
            assert!(is_ga100(device));
            assert_eq!(family(device), "ampere");
        }
        assert!(!is_ga100(0x2204));
    }
}
