//! The bytes stored in the 'vbios' registry value: the ROM image chain of one GPU (image.py).
//!
//! The file must pass the same checks as the Linux loader (payload.rs): exactly one NVIDIA legacy image, for this
//! GPU's PCI ID, with a zero checksum. The stored value starts at that legacy image and ends at the end of the
//! last image of its chain, following the NPDE extended lengths and last-image flags. Bytes before the legacy
//! image (a flash prefix) and after the chain are not stored.

use crate::families::is_ga100;
use crate::payload::{check_loadable_payload, check_payload, find_legacy, PayloadError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// What goes into the registry.
    pub data: Vec<u8>,
    pub family: &'static str,
    /// Offset of the legacy image in the file.
    pub start: usize,
    /// Offset just past the last image in the file.
    pub end: usize,
}

/// Little-endian u16 at `at`; callers have checked the bounds.
fn u16_at(data: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([data[at], data[at + 1]]))
}

/// End offset of the image chain that starts at `start` (PCIR/NPDS images, NPDE lengths and last flags).
pub fn chain_end(data: &[u8], start: usize) -> Result<usize, PayloadError> {
    let len = data.len();
    let mut at = start;
    loop {
        if at.checked_add(0x1A).is_none_or(|end| end > len) {
            return Err(PayloadError(format!(
                "the image chain is truncated at {at:#x} (no image marked last)"
            )));
        }
        let signature = u16_at(data, at);
        if signature != 0xAA55 && signature != 0x4E56 {
            return Err(PayloadError(format!(
                "unexpected ROM signature {signature:#06x} at {at:#x}"
            )));
        }
        // at + 0x1A <= len, so these sums stay far below usize::MAX.
        let header = at + u16_at(data, at + 0x18);
        let expected: &[u8] = if signature == 0xAA55 { b"PCIR" } else { b"NPDS" };
        if data.get(header..header + 4) != Some(expected) || header + 0x18 > len {
            return Err(PayloadError(format!(
                "missing PCI data structure for the image at {at:#x}"
            )));
        }
        let mut blocks = u16_at(data, header + 0x10);
        let mut last = data[header + 0x15] & 0x80 != 0;
        let npde = (header + u16_at(data, header + 0x0A) + 0xF) & !0xF;
        if data.get(npde..npde + 4) == Some(b"NPDE".as_slice())
            && npde + 0x0B <= len
            && matches!(u16_at(data, npde + 4), 0x100 | 0x101)
        {
            blocks = u16_at(data, npde + 8);
            if u16_at(data, npde + 6) >= 0x0B {
                last = data[npde + 0x0A] & 0x80 != 0;
            }
        }
        if blocks == 0 || at + blocks * 512 > len {
            return Err(PayloadError(format!("the image at {at:#x} has an invalid length")));
        }
        at += blocks * 512;
        if last {
            return Ok(at);
        }
    }
}

/// Validate a payload file for this GPU and return the chain to store.
pub fn extract_chain(data: &[u8], vendor: u16, device: u16) -> Result<Chain, PayloadError> {
    check_payload(data, vendor, device)?;
    if is_ga100(device) {
        return Err(PayloadError(
            "GA100 (A100/A30/A800) uses different GSP firmware and is not supported".to_string(),
        ));
    }
    let start = find_legacy(data, vendor, device)?;
    let end = chain_end(data, start)?;
    let chain = data[start..end].to_vec();
    // Same checks on the stored bytes: one legacy image, and the 512 KiB TU10x/TU11x, GA10x and AD10x capacity.
    let family = check_loadable_payload(&chain, vendor, device)?;
    Ok(Chain {
        data: chain,
        family,
        start,
        end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::GA10X_MAX_SIZE;
    use crate::testing::{chain, image, rom, ImageOptions, PREFIX, TRAILING};

    const RTX_5070: u16 = 0x2C09;
    const RTX_5070_TI: u16 = 0x2C05;
    const RTX_4090: u16 = 0x2684;
    const RTX_A2000: u16 = 0x2531;
    const A100: u16 = 0x20B5;
    const RTX_2060_SUPER: u16 = 0x1F06;

    fn error(result: Result<Chain, PayloadError>) -> String {
        match result {
            Ok(chain) => panic!("accepted: {} bytes", chain.data.len()),
            Err(PayloadError(message)) => message,
        }
    }

    fn data(blocks: u16, code_type: u8, last: bool) -> ImageOptions {
        ImageOptions {
            code_type,
            blocks,
            last,
            ..ImageOptions::default()
        }
    }

    #[test]
    fn prefix_and_trailing_bytes_are_dropped() {
        let file = rom(RTX_5070, 4);
        let result = extract_chain(&file, 0x10DE, RTX_5070).unwrap();
        assert_eq!(result.data, chain(RTX_5070, 4, ImageOptions::default()));
        assert_eq!(
            (result.start, result.end, result.family),
            (PREFIX, file.len() - TRAILING, "blackwell-gb20x")
        );
    }

    #[test]
    fn npde_length_and_last_flag_win_over_pcir() {
        // The legacy image's PCIR says 2 blocks; its NPDE extends it to 3. The PCIR last flag alone does not end
        // the chain.
        let mut file = image(
            RTX_5070,
            ImageOptions {
                npde_blocks: Some(3),
                pcir_last: Some(true),
                ..Default::default()
            },
        );
        file.extend(image(RTX_5070, data(2, 0xE0, true)));
        file.extend([0xFF; 512]);
        assert_eq!(extract_chain(&file, 0x10DE, RTX_5070).unwrap().data.len(), 5 * 512);
        // Without NPDE headers the PCIR lengths and last flag are used.
        let mut file = image(
            RTX_5070,
            ImageOptions {
                npde: false,
                ..Default::default()
            },
        );
        file.extend(image(
            RTX_5070,
            ImageOptions {
                npde: false,
                ..data(2, 0xE0, true)
            },
        ));
        let mut padded = file.clone();
        padded.extend([0u8; 512]);
        assert_eq!(extract_chain(&padded, 0x10DE, RTX_5070).unwrap().data, file);
    }

    #[test]
    fn chain_without_a_last_image_or_with_two_banks_is_refused() {
        let mut unterminated = image(RTX_5070, ImageOptions::default());
        unterminated.extend(image(RTX_5070, data(2, 0xE0, false)));
        assert!(error(extract_chain(&unterminated, 0x10DE, RTX_5070)).contains("truncated"));
        let two = chain(RTX_5070, 4, ImageOptions::default()).repeat(2);
        assert!(error(extract_chain(&two, 0x10DE, RTX_5070)).contains("exactly one"));
    }

    #[test]
    fn wrong_device_ga100_and_non_nvidia_are_refused() {
        assert!(error(extract_chain(&rom(RTX_5070, 4), 0x10DE, RTX_5070_TI)).contains("not 10de:2c05"));
        assert!(error(extract_chain(&rom(A100, 4), 0x10DE, A100)).contains("GA100"));
        assert!(error(extract_chain(&rom(RTX_5070, 4), 0x1002, RTX_5070)).contains("only NVIDIA"));
    }

    #[test]
    fn ada_and_ampere_chains_are_limited_to_512_kib_blackwell_is_not() {
        let fixed = 2 * 512 + 512; // legacy + EFI in front of the data image
        let fits = ((GA10X_MAX_SIZE - fixed) / 512) as u16;
        for (device, family) in [(RTX_4090, "ada"), (RTX_A2000, "ampere")] {
            let chain = extract_chain(&rom(device, fits), 0x10DE, device).unwrap();
            assert_eq!((chain.data.len(), chain.family), (GA10X_MAX_SIZE, family));
            assert!(
                error(extract_chain(&rom(device, fits + 1), 0x10DE, device)).contains("512 KiB"),
                "{family}"
            );
        }
        assert_eq!(
            extract_chain(&rom(RTX_5070, 1800), 0x10DE, RTX_5070)
                .unwrap()
                .data
                .len(),
            fixed + 1800 * 512
        );
    }

    #[test]
    fn turing_chains_end_at_the_type_0x70_image_or_at_the_last_data_image() {
        // Experimental: Turing ends its chain with a small type-0x70 image (NPDS flagged last, no NPDE).
        let mut tail = image(
            0,
            ImageOptions {
                npde: false,
                ..data(1, 0xE0, true)
            },
        );
        tail[0x54] = 0x70;
        let mut head = image(RTX_2060_SUPER, ImageOptions::default());
        head.extend(image(RTX_2060_SUPER, data(1, 3, false)));
        head.extend(image(RTX_2060_SUPER, data(1, 0xE0, false)));
        let mut default = head.clone();
        default.extend(image(RTX_2060_SUPER, data(4, 0xE0, false)));
        default.extend(&tail);
        let mut file = vec![0xFF; PREFIX];
        file.extend(&default);
        file.extend(vec![0xFF; TRAILING]);
        let result = extract_chain(&file, 0x10DE, RTX_2060_SUPER).unwrap();
        assert_eq!((result.data, result.family), (default, "turing"));
        // turing-e0-end-v1: the last data image carries both last-image flags and the tail is gone.
        let mut e0_end = head;
        e0_end.extend(image(
            RTX_2060_SUPER,
            ImageOptions {
                pcir_last: Some(true),
                ..data(4, 0xE0, true)
            },
        ));
        let mut file = e0_end.clone();
        file.extend(vec![0xFF; TRAILING]);
        assert_eq!(extract_chain(&file, 0x10DE, RTX_2060_SUPER).unwrap().data, e0_end);
        let fits = ((GA10X_MAX_SIZE - 2 * 512 - 512) / 512) as u16;
        assert!(
            error(extract_chain(&rom(RTX_2060_SUPER, fits + 1), 0x10DE, RTX_2060_SUPER))
                .contains("Turing TU10x/TU11x GSP accepts at most 512 KiB")
        );
    }

    #[test]
    fn texts_of_every_refusal() {
        // The exact PayloadError texts of payload.py and image.py.
        assert_eq!(
            error(extract_chain(&[0; 100], 0x10DE, RTX_5070)),
            "payload is 100 bytes; the loader accepts 512 B to 4 MiB"
        );
        assert_eq!(
            error(extract_chain(&rom(RTX_5070, 4), 0x10DE, 0x2330)),
            "PCI device 2330 is outside the supported Turing/Ampere/Ada/GB20x scope"
        );
        assert_eq!(
            error(extract_chain(&rom(RTX_5070, 4), 0x10DE, RTX_5070_TI)),
            "legacy image at 0x400 is for PCI ID 10de:2c09, not 10de:2c05"
        );
        assert_eq!(
            error(extract_chain(&[0xFF; 1024], 0x10DE, RTX_5070)),
            "payload has 0 legacy images; the driver needs exactly one"
        );
        let mut bad = rom(RTX_5070, 4);
        bad[PREFIX + 0x100] ^= 1;
        assert_eq!(
            error(extract_chain(&bad, 0x10DE, RTX_5070)),
            "legacy image at 0x400 has a nonzero checksum"
        );
        let mut bad = rom(RTX_5070, 4);
        bad[PREFIX + 0x50..PREFIX + 0x52].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            error(extract_chain(&bad, 0x10DE, RTX_5070)),
            "legacy image at 0x400 has an invalid length"
        );
        assert_eq!(
            error(extract_chain(
                &chain(RTX_5070, 4, ImageOptions::default()).repeat(2),
                0x10DE,
                RTX_5070
            )),
            "payload has 2 legacy images; the driver needs exactly one (a full ROM with two banks?); load a \
             single-chain payload instead: Vix Editor saves one as <name>-loader.bin next to an edited Turing, \
             Ampere or Ada copy, and its `normalize` command exports one from any ROM"
        );
        let fits = ((GA10X_MAX_SIZE - 2 * 512 - 512) / 512) as u16;
        assert_eq!(
            error(extract_chain(&rom(RTX_4090, fits + 1), 0x10DE, RTX_4090)),
            "Ada AD10x GSP accepts at most 512 KiB (524288 bytes) from the legacy image; this payload sends 524800 \
             bytes. Load an experimental relocated payload instead: Vix Editor saves one as <name>-loader.bin next \
             to an edited copy, or export it with `nvbios normalize edited.rom payload.bin --representation \
             ada-relocated-v2`."
        );
    }

    #[test]
    fn never_panics_on_damaged_payloads() {
        // Damaged synthetic payloads and random data through every check (overflow checks are on in tests).
        use crate::payload::{check_loadable_payload, find_legacy};
        use crate::testing::Rng;
        let mut rng = Rng::new(0x5EED);
        let bases = [
            rom(RTX_5070, 4),
            rom(RTX_2060_SUPER, 1),
            rom(RTX_4090, 2),
            chain(RTX_5070, 1, ImageOptions::default()),
            chain(
                RTX_A2000,
                3,
                ImageOptions {
                    npde: false,
                    ..ImageOptions::default()
                },
            ),
        ];
        let fields: [usize; 16] = [
            0x00, 0x01, 0x18, 0x19, 0x40, 0x4A, 0x4B, 0x50, 0x51, 0x54, 0x55, 0x64, 0x66, 0x68, 0x69, 0x6A,
        ];
        let values = [0u8, 1, 2, 0x0B, 0x0C, 0x18, 0x40, 0x7F, 0x80, 0xE0, 0xFF];
        for round in 0..6000 {
            let mut data = if round % 10 == 9 {
                let size = rng.below(3000);
                let mut random = rng.bytes(size);
                for block in (0..random.len().saturating_sub(1)).step_by(512) {
                    if rng.below(2) == 0 {
                        random[block..block + 2].copy_from_slice(&[0x55, 0xAA]);
                    }
                }
                random
            } else {
                rng.pick(&bases).clone()
            };
            for _ in 0..rng.below(4) {
                if data.is_empty() {
                    break;
                }
                let image = *rng.pick(&[0usize, 0x400, 0x800, 0xC00, 0xE00]);
                let at = (image + *rng.pick(&fields)) % data.len();
                data[at] = if rng.below(3) == 0 {
                    rng.byte()
                } else {
                    *rng.pick(&values)
                };
            }
            if rng.below(8) == 0 {
                data.truncate(rng.below(data.len() + 1));
            }
            for device in [RTX_5070, RTX_2060_SUPER, RTX_4090, RTX_A2000, A100, 0x1234] {
                let _ = find_legacy(&data, 0x10DE, device);
                let _ = check_loadable_payload(&data, 0x10DE, device);
                let _ = extract_chain(&data, 0x10DE, device);
            }
            for start in [
                0,
                0x400,
                rng.below(data.len() + 1),
                data.len(),
                usize::MAX,
                usize::MAX - 0x19,
            ] {
                let _ = chain_end(&data, start);
            }
        }
    }

    #[test]
    fn chain_end_texts_and_bounds() {
        let full = chain(RTX_5070, 4, ImageOptions::default());
        assert_eq!(chain_end(&full, 0), Ok(full.len()));
        for start in [full.len(), full.len() - 0x19, usize::MAX, usize::MAX - 0x19] {
            assert_eq!(
                chain_end(&full, start),
                Err(PayloadError(format!(
                    "the image chain is truncated at {start:#x} (no image marked last)"
                )))
            );
        }
        assert_eq!(
            chain_end(&full, 1),
            Err(PayloadError("unexpected ROM signature 0xffaa at 0x1".into()))
        );
        assert_eq!(
            chain_end(&[0; 0x20], 0),
            Err(PayloadError("unexpected ROM signature 0x0000 at 0x0".into()))
        );
        let mut header = full.clone();
        header[0x18] = 0xF0;
        assert_eq!(
            chain_end(&header, 0),
            Err(PayloadError("missing PCI data structure for the image at 0x0".into()))
        );
        let mut length = full.clone();
        length[0x68] = 0xFF; // NPDE length far beyond the data
        assert_eq!(
            chain_end(&length, 0),
            Err(PayloadError("the image at 0x0 has an invalid length".into()))
        );
    }
}
