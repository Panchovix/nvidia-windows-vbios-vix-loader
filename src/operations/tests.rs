//! The tests of tests/test_windows_loader.py that do not go through cli.main (ChainTests are in image.rs, the
//! Windows strings in devices.rs) and the VerifyTests of tests/test_windows_loader_verify.py, with the same inputs
//! and expected texts. Python's assertRaisesRegex patterns become ordered substrings (`.*` splits them).

use std::collections::{BTreeMap, BTreeSet};

use super::*;
use crate::devices::{REG_EXPAND_SZ, REG_MULTI_SZ, REG_NONE, REG_QWORD, REG_SZ};
use crate::testing::{
    adapter, binary as bin, chain, dword, platform, platform_with_version, raw, rom, sz, unrelated, FakeKey,
    ImageOptions, OnDelete, OnWrite, A100, PREFIX, RTX_2060_SUPER, RTX_4090, RTX_5070, RTX_5070_TI, RTX_A2000,
    TRAILING,
};

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn chain_of(device: u16, data_blocks: u16) -> Vec<u8> {
    chain(device, data_blocks, ImageOptions::default())
}

fn none() -> Selection {
    Selection::default()
}

/// The error text of a call that must fail.
fn error<T: std::fmt::Debug>(result: Result<T>) -> String {
    match result {
        Ok(value) => panic!("expected an error, got {value:?}"),
        Err(error) => error.message().to_string(),
    }
}

/// assertRaisesRegex with a pattern made of literal parts joined by `.*`.
fn assert_error<T: std::fmt::Debug>(result: Result<T>, pattern: &[&str]) {
    let text = error(result);
    let mut rest = text.as_str();
    for part in pattern {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => panic!("{text:?} does not match {pattern:?}"),
        }
    }
}

/// `report[a][b]...`.
fn at<'a>(report: &'a Map, path: &[&str]) -> &'a Value {
    let (first, rest) = path.split_first().expect("a path");
    let mut value = report
        .get(first)
        .unwrap_or_else(|| panic!("no {first:?} in the report"));
    for name in rest {
        value = value.get(name).unwrap_or_else(|| panic!("no {name:?} in {value:?}"));
    }
    value
}

fn text<'a>(report: &'a Map, path: &[&str]) -> &'a str {
    at(report, path)
        .as_str()
        .unwrap_or_else(|| panic!("{path:?} is not a text"))
}

fn object(entries: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (name, value) in entries {
        map.insert(name, value.clone());
    }
    Value::Object(map)
}

fn step_names(report: &Map) -> Vec<String> {
    let steps = at(report, &["steps"]).as_array().expect("steps");
    steps
        .iter()
        .map(|step| step.get("name").and_then(Value::as_str).expect("name").to_string())
        .collect()
}

/// `key.values` compared as a dict (order does not matter).
fn values(key: &FakeKey) -> BTreeMap<String, Decoded> {
    key.values().into_iter().collect()
}

fn with_unrelated(extra: &[(&str, Decoded)]) -> BTreeMap<String, Decoded> {
    let mut expected: BTreeMap<String, Decoded> = unrelated().into_iter().collect();
    for (name, value) in extra {
        expected.insert(name.to_string(), value.clone());
    }
    expected
}

fn names(key: &FakeKey) -> BTreeSet<String> {
    key.values().into_iter().map(|(name, _)| name).collect()
}

fn unrelated_names(extra: &[&str]) -> BTreeSet<String> {
    unrelated()
        .into_iter()
        .map(|(name, _)| name)
        .chain(extra.iter().map(|name| name.to_string()))
        .collect()
}

fn called(key: &FakeKey) -> BTreeSet<String> {
    key.calls().into_iter().map(|(_, name)| name).collect()
}

fn get_raw(key: &FakeKey, name: &str) -> Option<(u32, Vec<u8>)> {
    RegistryKey::get_raw(key, name).expect("a fake read")
}

fn load(fake: &dyn Platform, data: &[u8]) -> Result<Map> {
    super::load(fake, data, &none(), false)
}

fn dry_load(fake: &dyn Platform, data: &[u8]) -> Result<Map> {
    super::load(fake, data, &none(), true)
}

fn unload(fake: &dyn Platform) -> Result<Map> {
    super::unload(fake, &none(), false)
}

fn dry_unload(fake: &dyn Platform) -> Result<Map> {
    super::unload(fake, &none(), true)
}

fn first_status(fake: &dyn Platform) -> Map {
    let report = status(fake).expect("status");
    report[0].as_object().expect("an entry").clone()
}

// LoadUnloadTests

#[test]
fn load_writes_the_override_and_removes_patch_records_only() {
    let (fake, key) = platform(
        RTX_5070,
        vec![
            (FIRMWARE, dword(0x12)),
            ("romOverride00", bin(b"x")),
            ("romOverride7F", bin(b"y")),
        ],
    );
    let report = load(&fake, &rom(RTX_5070, 4)).unwrap();
    assert_eq!(key.value(VBIOS), Some(bin(&chain_of(RTX_5070, 4))));
    assert_eq!(key.value(FIRMWARE), Some(dword(0x12))); // the driver's default stays
    assert_eq!(key.value(BACKUP), Some(sz("0x00000012")));
    assert!(!key.values().iter().any(|(name, _)| name.starts_with("romOverride")));
    for (name, value) in unrelated() {
        assert_eq!(key.value(&name), Some(value));
    }
    let expected: BTreeSet<String> = [BACKUP, VBIOS, "romOverride00", "romOverride7F"]
        .iter()
        .map(|name| name.to_string())
        .collect();
    assert_eq!(called(&key), expected);
    assert_eq!(key.flushes(), 1);
    assert_eq!(text(&report, &["next_step"]), RESTART_HINT);
    assert_eq!(at(&report, &["payload", "dropped_prefix"]), &Value::from(PREFIX));
    assert_eq!(at(&report, &["payload", "dropped_trailing"]), &Value::from(TRAILING));
}

#[test]
fn unload_restores_a_previous_value() {
    let (fake, key) = platform(RTX_5070, vec![(FIRMWARE, dword(0x12))]);
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    assert_eq!(text(&unload(&fake).unwrap(), &["next_step"]), RESTART_HINT);
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x12))]));
}

#[test]
fn unload_deletes_a_value_that_was_absent() {
    let (fake, key) = platform(RTX_5070, vec![]);
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    assert_eq!(key.value(BACKUP), Some(sz(ABSENT)));
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[]));
}

#[test]
fn second_load_keeps_the_original_backup() {
    let (fake, key) = platform(RTX_5070, vec![(FIRMWARE, dword(0x12))]);
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    load(&fake, &rom(RTX_5070, 6)).unwrap();
    assert_eq!(key.value(BACKUP), Some(sz("0x00000012")));
    assert_eq!(key.value(VBIOS), Some(bin(&chain_of(RTX_5070, 6))));
    unload(&fake).unwrap();
    assert_eq!(key.value(FIRMWARE), Some(dword(0x12)));
}

#[test]
fn unload_without_our_backup_leaves_enable_gpu_firmware() {
    let (fake, key) = platform(RTX_5070, vec![(VBIOS, bin(b"old")), (FIRMWARE, dword(0x11))]);
    let report = unload(&fake).unwrap();
    assert_eq!(key.value(VBIOS), None);
    assert_eq!(key.value(FIRMWARE), Some(dword(0x11)));
    assert!(text(&report, &["message"]).contains("left as it is"));
    let (fake, key) = platform(RTX_5070, vec![]);
    let report = unload(&fake).unwrap();
    assert!(text(&report, &["message"]).contains("No override"));
    assert!(!report.contains_key("next_step")); // nothing changed, nothing to apply
    assert_eq!(key.calls(), vec![]);
}

#[test]
fn dry_run_writes_nothing_and_needs_no_admin() {
    for unload_it in [false, true] {
        let (fake, key) = platform(
            RTX_5070,
            vec![
                (FIRMWARE, dword(0x12)),
                (VBIOS, bin(b"old")),
                ("romOverride01", bin(b"x")),
            ],
        );
        fake.set_admin(false);
        let report = if unload_it {
            dry_unload(&fake)
        } else {
            dry_load(&fake, &rom(RTX_5070, 4))
        }
        .unwrap();
        assert!(at(&report, &["dry_run"]).truthy() && at(&report, &["steps"]).truthy());
        assert!(!report.contains_key("next_step"));
        assert_eq!((key.calls(), key.flushes(), fake.writable_opens()), (vec![], 0, 0));
    }
    let (fake, _) = platform(RTX_5070, vec![("romOverride01", bin(b"x"))]);
    fake.set_admin(false);
    let report = dry_load(&fake, &rom(RTX_5070, 4)).unwrap();
    let delete = object(&[("op", "delete".into()), ("name", "romOverride01".into())]);
    assert!(at(&report, &["steps"]).as_array().unwrap().contains(&delete));
}

#[test]
fn refusals_write_nothing() {
    let too_large =
        "EnableGpuFirmware holds a REG_BINARY value of 4097 bytes, too large to save (at most 4096); not changing it";
    // (device, values, driver version, admin, error pattern)
    type Case<'a> = (u16, Vec<(&'a str, Decoded)>, Option<&'a str>, bool, Vec<&'a str>);
    let cases: Vec<Case> = vec![
        (RTX_5070, vec![], Some("32.0.16.1656"), false, vec!["administrator"]),
        (RTX_5070, vec![], Some("32.0.15.6094"), true, vec!["560.94 is too old"]),
        (
            RTX_5070,
            vec![],
            Some("unknown"),
            true,
            vec!["cannot read the NVIDIA driver version"],
        ),
        (
            RTX_5070_TI,
            vec![],
            Some("32.0.16.1656"),
            true,
            vec!["payload refused", "not 10de:2c05"],
        ),
        (RTX_4090, vec![], Some("32.0.16.1656"), true, vec!["payload refused"]),
        (
            RTX_5070,
            vec![(FIRMWARE, bin(&vec![0; MAX_SAVED + 1]))],
            Some("32.0.16.1656"),
            true,
            vec![too_large],
        ),
        (
            RTX_5070,
            vec![(BACKUP, dword(0x12))],
            Some("32.0.16.1656"),
            true,
            vec!["run unload first"],
        ),
        (
            RTX_5070,
            vec![(BACKUP, sz("hex(1):3"))],
            Some("32.0.16.1656"),
            true,
            vec!["run unload first"],
        ),
    ];
    for (device, values, version, admin, message) in cases {
        let (fake, key) = platform_with_version(device, values, version);
        fake.set_admin(admin);
        assert_error(load(&fake, &rom(RTX_5070, 1100)), &message);
        assert_eq!(key.calls(), vec![], "{message:?}");
    }
    let (fake, key) = platform(RTX_5070, vec![(VBIOS, bin(b"old"))]);
    fake.set_admin(false);
    assert_error(unload(&fake), &["administrator"]);
    assert_eq!(key.calls(), vec![]);
}

#[test]
fn refusal_texts() {
    // The whole texts behind the patterns above (operations.py).
    let (fake, _) = platform_with_version(RTX_5070, vec![], Some("32.0.15.6094"));
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "NVIDIA driver 560.94 is too old; 616.56 or newer is required"
    );
    let (fake, _) = platform_with_version(RTX_5070, vec![], Some("unknown"));
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "cannot read the NVIDIA driver version of GPU 0 ('unknown')"
    );
    let (fake, _) = platform_with_version(RTX_5070, vec![], None);
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "cannot read the NVIDIA driver version of GPU 0 (None)"
    );
    let (fake, _) = platform(RTX_5070_TI, vec![]);
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "payload refused for GPU 0 (10de:2c05): legacy image at 0x400 is for PCI ID 10de:2c09, not 10de:2c05"
    );
    let (fake, _) = platform(A100, vec![]);
    assert_eq!(
        error(load(&fake, &rom(A100, 4))),
        "payload refused for GPU 0 (10de:20b5): GA100 (A100/A30/A800) uses different GSP firmware and is not supported"
    );
    let (fake, _) = platform(RTX_5070, vec![]);
    fake.set_admin(false);
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "administrator rights are required; run this from an elevated PowerShell"
    );
    let (fake, _) = platform(RTX_5070, vec![(BACKUP, sz("junk"))]);
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "VixLoaderPreviousEnableGpuFirmware holds an unexpected value; run unload first"
    );
}

#[test]
fn partial_write_failure_says_so_and_unload_recovers() {
    let (fake, key) = platform(RTX_5070, vec![]);
    let gpu = fake.adapters().unwrap()[0].clone();
    // FailingKey: set() of 'vbios' raises OSError(5, 'writing vbios failed: Access is denied').
    let key = FakeKey::new(key.values());
    key.on_set(|name, _| (name == VBIOS).then(|| Error::new("[Errno 5] writing vbios failed: Access is denied")));
    fake.set_key(&gpu.instance_id, key.clone());
    assert_error(
        load(&fake, &rom(RTX_5070, 4)),
        &["Access is denied; the registry may be partly changed", "unload"],
    );
    assert_eq!(key.value(BACKUP), Some(sz(ABSENT))); // written before the failure
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[]));
}

#[test]
fn partial_write_failure_text() {
    let (fake, key) = platform(RTX_5070, vec![]);
    key.on_set(|name, _| (name == VBIOS).then(|| Error::new("[Errno 5] writing vbios failed: Access is denied")));
    assert_eq!(
        error(load(&fake, &rom(RTX_5070, 4))),
        "[Errno 5] writing vbios failed: Access is denied; the registry may be partly changed: run status, then \
         unload, before restarting Windows"
    );
    assert_eq!(key.calls(), vec![("set".to_string(), BACKUP.to_string())]);
    assert_eq!((key.flushes(), key.closed()), (0, 1));
}

#[test]
fn other_tools_override_is_replaced_and_its_records_removed_by_unload() {
    let (fake, key) = platform(RTX_5070, vec![(VBIOS, bin(b"mvolt")), (FIRMWARE, dword(0x11))]);
    assert!(text(&load(&fake, &rom(RTX_5070, 4)).unwrap(), &["message"]).contains("another tool"));
    assert_eq!(key.value(FIRMWARE), Some(dword(0x11)));
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x11))]));
    let (fake, key) = platform(RTX_5070, vec![("romOverride00", bin(b"r")), (FIRMWARE, dword(0x11))]);
    let report = unload(&fake).unwrap();
    assert!(text(&report, &["message"]).contains("left as it is"));
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x11))]));
}

#[test]
fn malformed_values_are_reported_and_unload_still_removes_the_image() {
    let (fake, _) = platform(RTX_5070, vec![(FIRMWARE, raw(REG_DWORD, b"\x11\x00"))]);
    assert_eq!(
        text(&first_status(&fake), &["override", "enable_gpu_firmware"]),
        "unexpected REG_DWORD 2 bytes: 11 00"
    );
    let (fake, key) = platform(
        RTX_5070,
        vec![(VBIOS, bin(b"x")), (BACKUP, sz("junk")), (FIRMWARE, dword(0x11))],
    );
    assert!(text(&unload(&fake).unwrap(), &["message"]).contains("unexpected value"));
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x11))]));
}

#[test]
fn read_back_mismatch_is_an_error() {
    let (fake, key) = platform(RTX_5070, vec![]);
    key.set_drop_writes(true);
    assert_error(load(&fake, &rom(RTX_5070, 4)), &["read back"]);

    // KeepsFirmware: EnableGpuFirmware is not deleted when 0.3.x's 0x11 is undone.
    let (fake, key) = platform(
        RTX_5070,
        vec![(VBIOS, bin(b"old")), (FIRMWARE, dword(0x11)), (BACKUP, sz(ABSENT))],
    );
    let key = FakeKey::new(key.values());
    key.on_delete(|name| {
        if name == FIRMWARE {
            OnDelete::Keep
        } else {
            OnDelete::Normal
        }
    });
    fake.set_key(&fake.adapters().unwrap()[0].instance_id, key.clone());
    assert_error(
        load(&fake, &rom(RTX_5070, 4)),
        &["read back differ from what was written; run unload"],
    );
    assert_eq!(key.value(BACKUP), Some(sz(ABSENT)));

    // DropsBackup: the saved value is not written. KeepsRecords: romOverride records are not deleted.
    for keeps_records in [false, true] {
        let values = if keeps_records {
            vec![("romOverride00", bin(b"r"))]
        } else {
            vec![]
        };
        let (fake, key) = platform(RTX_5070, values);
        let key = FakeKey::new(key.values());
        if keeps_records {
            key.on_delete(|name| {
                if name.starts_with("romOverride") {
                    OnDelete::Keep
                } else {
                    OnDelete::Normal
                }
            });
        } else {
            key.on_set_raw(|name, _, _| if name == BACKUP { OnWrite::Skip } else { OnWrite::Store });
        }
        fake.set_key(&fake.adapters().unwrap()[0].instance_id, key);
        assert_error(
            load(&fake, &rom(RTX_5070, 4)),
            &["read back differ from what was written; run unload"],
        );
    }
}

#[test]
fn load_and_unload_ask_for_a_windows_restart_and_restart_nothing() {
    assert_eq!(
        RESTART_HINT,
        "Restart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. With it on, \
         the change waits for the next restart, and nothing has to be repeated."
    );
    let (fake, _) = platform(RTX_5070, vec![]);
    assert!(!dry_load(&fake, &rom(RTX_5070, 4)).unwrap().contains_key("next_step"));
    for report in [load(&fake, &rom(RTX_5070, 4)), dry_unload(&fake), unload(&fake)] {
        let report = report.unwrap();
        assert!(!report.contains_key("restart"));
        let expected = if at(&report, &["dry_run"]).truthy() {
            None
        } else {
            Some(RESTART_HINT)
        };
        assert_eq!(report.get("next_step").and_then(Value::as_str), expected);
    }
}

// DefaultModeTests: load leaves EnableGpuFirmware to the driver's default mode; it only undoes the 0x11 that
// loader 0.3.x wrote.

#[test]
fn a_first_load_leaves_enable_gpu_firmware_as_it_is() {
    let cases: [(Option<Decoded>, &str); 3] = [
        (None, ABSENT),
        (Some(dword(0x12)), "0x00000012"),
        (Some(raw(REG_SZ, &utf16("1\u{0}"))), "hex(1):31,00,00,00"),
    ];
    for device in [RTX_4090, RTX_5070, RTX_A2000, RTX_2060_SUPER] {
        for (firmware, saved) in &cases {
            let values = firmware.iter().map(|value| (FIRMWARE, value.clone())).collect();
            let (fake, key) = platform(device, values);
            let before = get_raw(&key, FIRMWARE);
            let report = load(&fake, &rom(device, 4)).unwrap();
            assert_eq!(get_raw(&key, FIRMWARE), before);
            assert!(!called(&key).contains(FIRMWARE));
            assert_eq!(key.value(BACKUP), Some(sz(saved)));
            assert_eq!(key.value(VBIOS), Some(bin(&chain_of(device, 4))));
            assert_eq!(step_names(&report), [BACKUP, VBIOS]);
            for name in ["message", "restores_firmware", "keeps_gsp_firmware_mode"] {
                assert!(!report.contains_key(name), "{name}");
            }
            assert_eq!(at(&report, &["after", "gsp_forced_by_old_loader"]), &Value::Bool(false));
            assert_eq!(text(&report, &["next_step"]), RESTART_HINT);
            unload(&fake).unwrap();
            assert_eq!(get_raw(&key, FIRMWARE), before);
            let extra: &[&str] = if firmware.is_some() { &[FIRMWARE] } else { &[] };
            assert_eq!(names(&key), unrelated_names(extra), "{device:04x} {saved}");
        }
    }
}

#[test]
fn keys_left_by_0_3_2_get_the_saved_value_back() {
    // vbios, EnableGpuFirmware = 0x11 and the value saved before it, as loader 0.3.2 left them.
    let cases = [
        (
            ABSENT,
            object(&[("op", "delete".into()), ("name", FIRMWARE.into())]),
            None,
        ),
        (
            "0x00000012",
            object(&[
                ("op", "set".into()),
                ("name", FIRMWARE.into()),
                ("type", "REG_DWORD".into()),
                ("value", "0x12".into()),
            ]),
            Some((REG_DWORD, b"\x12\0\0\0".to_vec())),
        ),
        (
            "hex(1):31,00,00,00",
            object(&[
                ("op", "set".into()),
                ("name", FIRMWARE.into()),
                ("type", "REG_SZ".into()),
                ("value", "'1' (4 bytes)".into()),
                ("data", "31000000".into()),
            ]),
            Some((REG_SZ, b"1\0\0\0".to_vec())),
        ),
    ];
    for (saved_text, step, restored) in cases {
        let (fake, key) = platform(
            RTX_5070,
            vec![
                (VBIOS, bin(&chain_of(RTX_5070, 4))),
                (FIRMWARE, dword(0x11)),
                (BACKUP, sz(saved_text)),
            ],
        );
        fake.set_admin(false);
        let status = first_status(&fake);
        assert_eq!(
            at(&status, &["override", "gsp_forced_by_old_loader"]),
            &Value::Bool(true)
        );
        let shown = text(&status, &["override", "saved_previous_enable_gpu_firmware"]).to_string();
        let report = dry_load(&fake, &rom(RTX_5070, 6)).unwrap();
        let set_vbios = object(&[
            ("op", "set".into()),
            ("name", VBIOS.into()),
            ("type", "REG_BINARY".into()),
            ("size", chain_of(RTX_5070, 6).len().into()),
            ("sha256", at(&report, &["payload", "sha256"]).clone()),
        ]);
        assert_eq!(at(&report, &["steps"]), &Value::Array(vec![set_vbios, step]));
        assert_eq!(text(&report, &["restores_firmware"]), shown);
        assert_eq!(
            text(&report, &["message"]),
            format!(
                "EnableGpuFirmware goes back to its value before the first load: {shown}. Loader 0.3.2 or older set \
                 it to 0x11, which turns on the driver's GSP firmware mode; this version no longer turns that mode on."
            )
        );
        assert_eq!(key.calls(), vec![]);
        fake.set_admin(true);
        let report = load(&fake, &rom(RTX_5070, 6)).unwrap();
        assert_eq!(get_raw(&key, FIRMWARE), restored);
        assert_eq!(key.value(BACKUP), Some(sz(saved_text))); // kept: unload restores it again
        assert_eq!(key.value(VBIOS), Some(bin(&chain_of(RTX_5070, 6))));
        assert_eq!(at(&report, &["after", "gsp_forced_by_old_loader"]), &Value::Bool(false));
        assert_eq!(text(&report, &["next_step"]), RESTART_HINT);
        let report = load(&fake, &rom(RTX_5070, 4)).unwrap(); // nothing left to undo
        assert_eq!(step_names(&report), [VBIOS]);
        assert!(!report.contains_key("message"));
        unload(&fake).unwrap();
        assert_eq!(get_raw(&key, FIRMWARE), restored);
        let extra: &[&str] = if restored.is_some() { &[FIRMWARE] } else { &[] };
        assert_eq!(names(&key), unrelated_names(extra));
    }
}

#[test]
fn a_0x11_set_before_the_first_load_stays_with_a_note() {
    let note = "GSP firmware mode stays on because EnableGpuFirmware was already 0x11 before the first load (for \
                example set by mVolt+). For the driver's default mode, run unload (it puts 0x11 back), delete \
                EnableGpuFirmware, load again and restart Windows.";
    let (fake, key) = platform(RTX_5070, vec![(VBIOS, bin(b"mvolt")), (FIRMWARE, dword(0x11))]);
    let report = load(&fake, &rom(RTX_5070, 4)).unwrap();
    assert_eq!(
        (
            at(&report, &["replaces_other_override"]),
            at(&report, &["keeps_gsp_firmware_mode"])
        ),
        (&Value::Bool(true), &Value::Bool(true))
    );
    assert!(text(&report, &["message"]).starts_with("An override written by another tool"));
    assert!(text(&report, &["message"]).ends_with(&format!("(0x11). {note}")));
    assert_eq!(key.value(BACKUP), Some(sz("0x00000011")));
    assert_eq!(get_raw(&key, FIRMWARE), Some((REG_DWORD, b"\x11\0\0\0".to_vec())));
    assert_eq!(at(&report, &["after", "gsp_forced_by_old_loader"]), &Value::Bool(false));
    for backup in ["0x00000011", "hex(4):11,00,00,00"] {
        // also an exact copy of the same DWORD
        key.insert(BACKUP, sz(backup));
        let report = load(&fake, &rom(RTX_5070, 6)).unwrap();
        assert_eq!(
            (text(&report, &["message"]), at(&report, &["keeps_gsp_firmware_mode"])),
            (note, &Value::Bool(true))
        );
        assert!(!report.contains_key("restores_firmware"));
        assert_eq!(step_names(&report), [VBIOS]);
        assert_eq!(get_raw(&key, FIRMWARE), Some((REG_DWORD, b"\x11\0\0\0".to_vec())));
    }
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x11))]));
    // As the note says: deleted while no override is loaded, it is saved as absent and unload leaves it deleted.
    key.remove(FIRMWARE);
    assert!(!load(&fake, &rom(RTX_5070, 4)).unwrap().contains_key("message"));
    assert_eq!(key.value(BACKUP), Some(sz(ABSENT)));
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[]));
}

// RawFirmwareTests: EnableGpuFirmware values that are not a 4-byte REG_DWORD (set by another tool or by hand):
// load saves the exact type and bytes in .reg notation and leaves the value, and unload writes them back.

/// (type, data, backup text or None) of the Python test EVERY_TYPE.
fn every_type() -> Vec<(u32, Vec<u8>, Option<&'static str>)> {
    let mut big = Vec::new();
    for _ in 0..16 {
        big.extend(0..=255u8);
    }
    vec![
        (REG_SZ, utf16("17\u{0}"), Some("hex(1):31,00,37,00,00,00")),
        (REG_SZ, utf16("1\u{0}7\u{0}"), Some("hex(1):31,00,00,00,37,00,00,00")), // data after a NUL
        (REG_SZ, b"".to_vec(), Some("hex(1):")),                                 // empty, no terminator
        (REG_SZ, b"\0\0".to_vec(), Some("hex(1):00,00")),
        (REG_SZ, b"1".to_vec(), Some("hex(1):31")),        // odd size
        (REG_SZ, utf16("17"), Some("hex(1):31,00,37,00")), // no terminator
        (REG_SZ, b"1\x007\x00\x00".to_vec(), Some("hex(1):31,00,37,00,00")), // odd size, no terminator
        (REG_EXPAND_SZ, utf16("%a%"), Some("hex(2):25,00,61,00,25,00")),
        (REG_EXPAND_SZ, utf16("%SystemRoot%\u{0}"), None),
        (
            REG_MULTI_SZ,
            utf16("a\u{0}bc\u{0}\u{0}"),
            Some("hex(7):61,00,00,00,62,00,63,00,00,00,00,00"),
        ),
        (REG_MULTI_SZ, utf16("a"), Some("hex(7):61,00")),
        (
            REG_QWORD,
            0x11u64.to_le_bytes().to_vec(),
            Some("hex(b):11,00,00,00,00,00,00,00"),
        ),
        (REG_BINARY, b"\x11".to_vec(), Some("hex(3):11")),
        (REG_NONE, b"".to_vec(), Some("hex(0):")),
        (REG_NONE, b"\x11\0\0\0".to_vec(), Some("hex(0):11,00,00,00")),
        (REG_DWORD, b"\x11\0".to_vec(), Some("hex(4):11,00")),
        (
            REG_DWORD,
            0x12u64.to_le_bytes().to_vec(),
            Some("hex(4):12,00,00,00,00,00,00,00"),
        ),
        (0x12345, b"\xff".to_vec(), Some("hex(12345):ff")),
        (REG_BINARY, big, None), // the largest saved
    ]
}

/// BACKUP texts load refuses ("run unload first") and unload does not restore from.
fn malformed_backups() -> Vec<String> {
    let mut texts: Vec<String> = [
        "junk",
        "",
        " absent",
        "hex(1):3",
        "hex(1):31,0",
        "hex(1)31,00",
        "hex(01):31",
        "hex(1):31,,00",
        "hex(1):31,00,",
        "HEX(1):31",
        "hex(1):3g",
        "hex(1):3G",
        "hex(100000000):",
        "hex():31",
        "hex(1): 31",
        "hex(1):3100",
        "hex(-1):31",
        "-1",
        "0x100000000",
    ]
    .iter()
    .map(|text| text.to_string())
    .collect();
    texts.push(format!("hex(3):{}", vec!["00"; MAX_SAVED + 1].join(",")));
    texts
}

#[test]
fn every_type_is_saved_exactly_and_unload_puts_it_back() {
    for (kind, data, expected_text) in every_type() {
        let (fake, key) = platform(RTX_5070, vec![(FIRMWARE, raw(kind, &data))]);
        assert_eq!(get_raw(&key, FIRMWARE), Some((kind, data.clone())));
        let report = load(&fake, &rom(RTX_5070, 4)).unwrap();
        let bytes: Vec<String> = data.iter().map(|byte| format!("{byte:02x}")).collect();
        let backup = format!("hex({kind:x}):{}", bytes.join(","));
        assert_eq!(backup, expected_text.unwrap_or(&backup));
        assert_eq!(key.value(BACKUP), Some(sz(&backup)));
        assert_eq!(get_raw(&key, FIRMWARE), Some((kind, data.clone()))); // left as it is
        let raw_backup = object(&[
            ("type", type_name(kind).into()),
            ("size", data.len().into()),
            ("value", value_text(kind, &data).into()),
            ("backup", backup.as_str().into()),
        ]);
        assert_eq!(at(&report, &["raw_backup"]), &raw_backup);
        assert!(!report.contains_key("message"));
        assert!(!report.contains_key("replaces_other_override"));
        assert_eq!(
            text(&report, &["after", "saved_previous_enable_gpu_firmware"]),
            format!("{} {}", type_name(kind), value_text(kind, &data))
        );
        load(&fake, &rom(RTX_5070, 6)).unwrap(); // a second load keeps the saved value
        assert_eq!(key.value(BACKUP), Some(sz(&backup)));
        let report = unload(&fake).unwrap();
        assert_eq!(get_raw(&key, FIRMWARE), Some((kind, data.clone())));
        assert_eq!(names(&key), unrelated_names(&[FIRMWARE]));
        for (name, value) in unrelated() {
            assert_eq!(key.value(&name), Some(value));
        }
        assert!(!report.contains_key("message"));
    }
}

#[test]
fn status_shows_the_type_and_the_saved_value() {
    let qword = 0x11u64.to_le_bytes();
    let (fake, _) = platform(RTX_5070, vec![(FIRMWARE, raw(REG_QWORD, &qword))]);
    assert_eq!(
        text(&first_status(&fake), &["override", "enable_gpu_firmware"]),
        "unexpected REG_QWORD 0x11"
    );
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    let status = first_status(&fake);
    assert_eq!(
        (
            text(&status, &["override", "enable_gpu_firmware"]),
            text(&status, &["override", "saved_previous_enable_gpu_firmware"])
        ),
        ("unexpected REG_QWORD 0x11", "REG_QWORD 0x11")
    );
    // (The `vbios-loader status` line of this test belongs to the command line tests.)
    let cases: Vec<(u32, Vec<u8>, String)> = vec![
        (REG_SZ, utf16("17\u{0}"), "unexpected REG_SZ '17' (6 bytes)".into()),
        (
            REG_SZ,
            utf16("1\u{0}7\u{0}"),
            "unexpected REG_SZ '1\\x007' (8 bytes)".into(),
        ),
        (
            REG_MULTI_SZ,
            utf16("a\u{0}b\u{0}\u{0}"),
            "unexpected REG_MULTI_SZ ['a', 'b'] (10 bytes)".into(),
        ),
        (REG_EXPAND_SZ, vec![], "unexpected REG_EXPAND_SZ '' (0 bytes)".into()),
        (REG_NONE, vec![], "unexpected REG_NONE 0 bytes".into()),
        (REG_BINARY, b"\x11".to_vec(), "unexpected REG_BINARY 1 byte: 11".into()),
        (
            REG_BINARY,
            vec![0; 20],
            format!("unexpected REG_BINARY 20 bytes: {}...", "00 ".repeat(16)),
        ),
        (0x12345, vec![], "unexpected type 74565 0 bytes".into()),
    ];
    for (kind, data, shown) in cases {
        let (fake, _) = platform(RTX_5070, vec![(FIRMWARE, raw(kind, &data))]);
        assert_eq!(text(&first_status(&fake), &["override", "enable_gpu_firmware"]), shown);
    }
}

#[test]
fn backups_written_by_0_3_1_still_unload() {
    let cases = [
        ("0x00000012", Some((REG_DWORD, b"\x12\0\0\0".to_vec()))),
        ("0x00000000", Some((REG_DWORD, vec![0; 4]))),
        ("0xffffffff", Some((REG_DWORD, vec![0xff; 4]))),
        (ABSENT, None),
    ];
    for (saved_text, expected) in cases {
        let (fake, key) = platform(
            RTX_5070,
            vec![
                (VBIOS, bin(&chain_of(RTX_5070, 4))),
                (BACKUP, sz(saved_text)),
                (FIRMWARE, dword(0x11)),
            ],
        );
        load(&fake, &rom(RTX_5070, 6)).unwrap(); // accepted, and kept as it is
        assert_eq!(key.value(BACKUP), Some(sz(saved_text)));
        assert_eq!(get_raw(&key, FIRMWARE), expected); // 0.3.x's 0x11 is undone already
        let step = match &expected {
            None => object(&[("op", "delete".into()), ("name", FIRMWARE.into())]),
            Some(_) => object(&[
                ("op", "set".into()),
                ("name", FIRMWARE.into()),
                ("type", "REG_DWORD".into()),
                ("value", format!("{:#04x}", parse_int(saved_text, 16).unwrap()).into()),
            ]),
        };
        assert_eq!(at(&dry_unload(&fake).unwrap(), &["steps"]).as_array().unwrap()[0], step);
        unload(&fake).unwrap();
        assert_eq!(get_raw(&key, FIRMWARE), expected);
        let extra: &[&str] = if expected.is_some() { &[FIRMWARE] } else { &[] };
        assert_eq!(names(&key), unrelated_names(extra));
    }
}

#[test]
fn malformed_backups_are_refused_and_not_restored() {
    for backup in malformed_backups() {
        let (fake, key) = platform(
            RTX_5070,
            vec![(VBIOS, bin(b"x")), (BACKUP, sz(&backup)), (FIRMWARE, dword(0x11))],
        );
        assert_error(
            load(&fake, &rom(RTX_5070, 4)),
            &["holds an unexpected value; run unload first"],
        );
        assert_eq!(key.calls(), vec![], "{backup:?}");
        assert!(text(&unload(&fake).unwrap(), &["message"])
            .contains("unexpected value; EnableGpuFirmware is left as it is"));
        assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, dword(0x11))]));
    }
}

#[test]
fn dry_runs_show_the_steps() {
    let qword = 0x11u64.to_le_bytes();
    let (fake, key) = platform(RTX_5070, vec![(FIRMWARE, raw(REG_QWORD, &qword))]);
    fake.set_admin(false);
    let report = dry_load(&fake, &rom(RTX_5070, 4)).unwrap();
    let expected = object(&[
        ("op", "set".into()),
        ("name", BACKUP.into()),
        ("type", "REG_SZ".into()),
        ("value", "hex(b):11,00,00,00,00,00,00,00".into()),
    ]);
    assert_eq!(at(&report, &["steps"]).as_array().unwrap()[0], expected);
    assert_eq!(step_names(&report), [BACKUP, VBIOS]);
    assert!(!report.contains_key("message"));
    assert_eq!(key.calls(), vec![]);
    // (The `load --dry-run` output of this test belongs to the command line tests.)

    let (fake, key) = platform(
        RTX_5070,
        vec![
            (VBIOS, bin(b"x")),
            (FIRMWARE, dword(0x11)),
            (BACKUP, sz("hex(1):31,00,00,00,37,00")),
        ],
    );
    fake.set_admin(false);
    let report = dry_unload(&fake).unwrap();
    let expected = Value::Array(vec![
        object(&[
            ("op", "set".into()),
            ("name", FIRMWARE.into()),
            ("type", "REG_SZ".into()),
            ("value", "'1\\x007' (6 bytes)".into()),
            ("data", "310000003700".into()),
        ]),
        object(&[("op", "delete".into()), ("name", VBIOS.into())]),
        object(&[("op", "delete".into()), ("name", BACKUP.into())]),
    ]);
    assert_eq!(at(&report, &["steps"]), &expected);
    assert_eq!(key.calls(), vec![]);

    // A long saved value is shortened in the step lines; the JSON steps keep all of it.
    let (fake, _) = platform(RTX_5070, vec![(FIRMWARE, bin(&vec![0; MAX_SAVED]))]);
    fake.set_admin(false);
    let report = dry_load(&fake, &rom(RTX_5070, 4)).unwrap();
    let step = &at(&report, &["steps"]).as_array().unwrap()[0];
    let value = step.get("value").and_then(Value::as_str).unwrap();
    assert_eq!(value, format!("hex(3):{}", vec!["00"; MAX_SAVED].join(",")));
}

#[test]
fn another_tools_override_is_noted_and_a_non_dword_value_kept() {
    let (fake, key) = platform(
        RTX_5070,
        vec![(VBIOS, bin(b"mvolt")), (FIRMWARE, raw(REG_SZ, b"\x11\0"))],
    );
    let report = load(&fake, &rom(RTX_5070, 4)).unwrap();
    assert_eq!(at(&report, &["replaces_other_override"]), &Value::Bool(true));
    assert_eq!(
        text(&report, &["message"]),
        "An override written by another tool (for example mVolt+) is replaced and will not be restored; unload \
         restores EnableGpuFirmware to its current value (REG_SZ '\\x11' (2 bytes))."
    );
    assert_eq!(get_raw(&key, FIRMWARE), Some((REG_SZ, b"\x11\0".to_vec())));
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, raw(REG_SZ, b"\x11\0"))]));
}

#[test]
fn a_restore_that_reads_back_differently_keeps_the_saved_value() {
    let (fake, key) = platform(RTX_5070, vec![(FIRMWARE, raw(REG_SZ, b"1\0"))]);
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    let gpu = fake.adapters().unwrap()[0].clone();
    // AlteringKey: a non-DWORD EnableGpuFirmware is written with two more bytes.
    let altering = FakeKey::new(key.values());
    altering.on_set_raw(|name, kind, data| {
        if name == FIRMWARE && kind != REG_DWORD {
            OnWrite::Replace(kind, [data, b"\0\0"].concat())
        } else {
            OnWrite::Store
        }
    });
    fake.set_key(&gpu.instance_id, altering.clone());
    assert_eq!(
        error(unload(&fake)),
        "EnableGpuFirmware read back differs from what was expected; VixLoaderPreviousEnableGpuFirmware is kept, \
         run unload again"
    );
    assert_eq!(altering.value(BACKUP), Some(sz("hex(1):31,00")));
    assert_eq!(altering.value(VBIOS), None);
    let key = FakeKey::new(altering.values());
    fake.set_key(&gpu.instance_id, key.clone());
    unload(&fake).unwrap();
    assert_eq!(values(&key), with_unrelated(&[(FIRMWARE, raw(REG_SZ, b"1\0"))]));
}

// SelectionTests (test_windows_strings is in devices.rs)

#[test]
fn selection_by_index_or_location_and_ambiguity() {
    let gpus = [
        adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
        adapter(1, RTX_5070, Some("02:00.0"), Some("32.0.16.1656")),
    ];
    let by_gpu = |gpu| Selection {
        gpu: Some(gpu),
        ..Selection::default()
    };
    let by_pci = |pci: &str| Selection {
        pci: Some(pci.to_string()),
        ..Selection::default()
    };
    assert_eq!(select(&gpus, &by_gpu(1)).unwrap().index, 1);
    assert_eq!(select(&gpus, &by_pci("0000:02:00.0")).unwrap().index, 1);
    assert_eq!(select(&gpus, &by_pci("2:0.0")).unwrap().index, 1);
    assert_eq!(select(&gpus[..1], &none()).unwrap().index, 0);
    let both = Selection {
        gpu: Some(0),
        pci: Some("01:00.0".to_string()),
        instance_id: None,
    };
    let cases = [
        (none(), "2 NVIDIA GPUs"),
        (by_gpu(5), "no NVIDIA GPU matches"),
        (both, "not both"),
        (by_pci("01:00"), "not a PCI location"),
        (by_pci("0001:01:00.0"), "not a PCI location"),
    ];
    for (selection, message) in cases {
        assert_error(select(&gpus, &selection), &[message]);
    }
    let same = [
        adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
        adapter(1, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
    ];
    assert_error(select(&same, &by_pci("01:00.0")), &["more than one"]);
}

#[test]
fn selection_texts() {
    let gpus = [
        adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
        adapter(1, RTX_5070, None, Some("32.0.16.1656")),
    ];
    let by_pci = |pci: &str| Selection {
        pci: Some(pci.to_string()),
        ..Selection::default()
    };
    assert_eq!(
        error(select(&gpus, &by_pci("01:00"))),
        "'01:00' is not a PCI location such as 01:00.0"
    );
    assert_eq!(
        error(select(&gpus, &by_pci("it's"))),
        "\"it's\" is not a PCI location such as 01:00.0"
    );
    assert_eq!(
        error(select(&gpus, &none())),
        "2 NVIDIA GPUs found; choose one with --gpu INDEX or --pci BUS:DEV.FN"
    );
    assert_eq!(
        error(select(&[], &none())),
        "no NVIDIA GPU matches that selection; run status to list them"
    );
    let by_gpu = |gpu| Selection {
        gpu: Some(gpu),
        ..Selection::default()
    };
    assert_eq!(select(&gpus, &by_gpu(1)).unwrap().index, 1);
    assert!(select(&gpus, &by_gpu(-1)).is_err());
    assert!(select(&gpus, &by_gpu(i128::MAX)).is_err());
    let by_id = Selection {
        instance_id: Some(gpus[1].instance_id.clone()),
        ..Selection::default()
    };
    assert_eq!(select(&gpus, &by_id).unwrap().index, 1);
    let three = Selection {
        gpu: Some(0),
        pci: None,
        instance_id: Some("x".to_string()),
    };
    assert_eq!(error(select(&gpus, &three)), "use either --gpu or --pci, not both");
}

// The JSON part of CliTests.test_status_flags_gpus_left_in_gsp_firmware_mode_by_0_3_x, through status().

#[test]
fn status_flags_gpus_left_in_gsp_firmware_mode_by_0_3_x() {
    let old = || vec![(VBIOS, bin(b"x")), (FIRMWARE, dword(0x11))];
    let with = |extra: Vec<(&'static str, Decoded)>| {
        let mut values = old();
        values.extend(extra);
        values
    };
    let cases = [
        (with(vec![(BACKUP, sz(ABSENT))]), true),
        (with(vec![(BACKUP, sz("hex(b):12,00,00,00,00,00,00,00"))]), true),
        (with(vec![(BACKUP, sz("0x00000011"))]), false), // 0x11 before the first load
        (with(vec![(BACKUP, sz("junk"))]), false),       // unload leaves EnableGpuFirmware
        (old(), false),                                  // mVolt+, no saved value
        (with(vec![(BACKUP, sz(ABSENT)), (FIRMWARE, dword(0x12))]), false),
        (
            with(vec![(BACKUP, sz(ABSENT)), (FIRMWARE, raw(REG_DWORD, b"\x11\0"))]),
            false,
        ),
    ];
    for (values, flagged) in cases {
        let (fake, _) = platform(RTX_5070, values);
        assert_eq!(
            at(&first_status(&fake), &["override", "gsp_forced_by_old_loader"]),
            &Value::Bool(flagged)
        );
    }
}

#[test]
fn status_report_and_read_errors() {
    let (fake, key) = platform(RTX_5070, vec![("romOverride02", bin(b"x"))]);
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    key.insert("romOverride02", bin(b"x"));
    let entry = first_status(&fake);
    let keys: Vec<&str> = entry.keys().collect();
    assert_eq!(
        keys,
        [
            "index",
            "name",
            "pci_id",
            "subsystem",
            "location",
            "instance_id",
            "family",
            "driver_version",
            "nvidia_driver_version",
            "driver_supported",
            "driver_key",
            "override"
        ]
    );
    assert_eq!(text(&entry, &["pci_id"]), "10de:2c09");
    assert_eq!(text(&entry, &["subsystem"]), "205e1043");
    assert_eq!(text(&entry, &["nvidia_driver_version"]), "616.56");
    assert_eq!(text(&entry, &["family"]), "blackwell-gb20x");
    assert_eq!(
        at(&entry, &["override", "vbios", "size"]),
        &Value::from(chain_of(RTX_5070, 4).len())
    );
    assert_eq!(at(&entry, &["override", "enable_gpu_firmware"]), &Value::Null);
    assert_eq!(
        text(&entry, &["override", "saved_previous_enable_gpu_firmware"]),
        "absent"
    );
    assert_eq!(
        at(&entry, &["override", "rom_override_records"]),
        &Value::Array(vec!["romOverride02".into()])
    );
    assert_eq!(key.closed(), 2); // one key for load, one for status
                                 // A key that cannot be read is reported, not raised.
    key.on_get(|name| (name == BACKUP).then(|| Error::new("[WinError 5] Access is denied")));
    let entry = first_status(&fake);
    assert_eq!(at(&entry, &["override"]), &Value::Null);
    assert_eq!(
        text(&entry, &["error"]),
        "cannot read the driver key: [WinError 5] Access is denied"
    );
    let gpu = fake.adapters().unwrap()[0].clone();
    fake.fail_open(
        &gpu.instance_id,
        Error::new(format!("device {} is no longer present", gpu.instance_id)),
    );
    let entry = first_status(&fake);
    assert_eq!(
        text(&entry, &["error"]),
        format!(
            "cannot read the driver key: device {} is no longer present",
            gpu.instance_id
        )
    );
    fake.fail_adapters(Some(Error::new("[WinError 13] The data is invalid.")));
    assert_eq!(error(status(&fake)), "[WinError 13] The data is invalid.");
}

// VerifyTests

/// BIG of the Python tests: longer than what NvAPI returns.
fn big() -> Vec<u8> {
    let mut data = Vec::with_capacity(256 * 4001);
    for _ in 0..4001 {
        data.extend(0..=255u8);
    }
    data
}

/// VerifyTests.verify: read-only, no admin, two reads.
fn verify_with(values: Vec<(&str, Decoded)>, image: &[u8]) -> Map {
    let (fake, key) = platform(RTX_5070, values);
    fake.set_image(image.to_vec());
    fake.set_admin(false);
    let report = verify(&fake, &none()).unwrap();
    assert_eq!((key.calls(), fake.writable_opens()), (vec![], 0)); // read-only, no admin
    assert_eq!(fake.reads().len(), 2);
    report
}

#[test]
fn the_loaded_override_is_active() {
    let (fake, _) = platform(RTX_5070, vec![]);
    fake.set_image(chain_of(RTX_5070, 4));
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    let report = verify(&fake, &none()).unwrap();
    assert_eq!(
        (
            text(&report, &["state"]),
            at(&report, &["match"]),
            at(&report, &["compared"]),
            at(&report, &["first_difference"])
        ),
        (
            "override-active",
            &Value::Bool(true),
            &Value::from("full"),
            &Value::Null
        )
    );
    let mut configured = at(&report, &["configured"]).as_object().unwrap().clone();
    configured.insert("truncated", false);
    assert_eq!(at(&report, &["readback"]), &Value::Object(configured));
    assert_eq!(
        at(&report, &["configured", "size"]),
        &Value::from(chain_of(RTX_5070, 4).len())
    );
    assert!(text(&report, &["message"]).contains("verified byte by byte"));
    let stored = chain_of(RTX_5070, 4);
    let report = verify_with(vec![(VBIOS, bin(&stored)), ("romOverride00", bin(b"r"))], &stored);
    assert_eq!(text(&report, &["state"]), "override-active");
    assert!(!text(&report, &["message"]).contains("romOverride")); // the records note is only for no-override
}

#[test]
fn a_different_image_is_not_active_with_the_first_difference() {
    let stored = chain_of(RTX_5070, 4);
    let mut running = stored.clone();
    running[0x123] = 0;
    let report = verify_with(vec![(VBIOS, bin(&stored))], &running);
    assert_eq!(
        (
            text(&report, &["state"]),
            at(&report, &["match"]),
            at(&report, &["first_difference"])
        ),
        ("override-not-active", &Value::Bool(false), &Value::from(0x123usize))
    );
    assert_eq!(
        text(&report, &["message"]),
        "The driver is NOT using the stored override (first difference at offset 0x123). If it was just loaded, \
         restart Windows; otherwise the driver may have rejected it and is running the card's own VBIOS."
    );
    let report = verify_with(vec![(VBIOS, bin(&stored))], &stored[..1024]);
    assert_eq!(
        (text(&report, &["state"]), at(&report, &["first_difference"])),
        ("override-not-active", &Value::from(1024usize))
    );
}

#[test]
fn without_an_override_the_card_vbios_runs() {
    let report = verify_with(vec![], b"card image");
    assert_eq!(
        (
            text(&report, &["state"]),
            at(&report, &["configured"]),
            at(&report, &["match"])
        ),
        ("no-override", &Value::Null, &Value::Null)
    );
    assert_eq!(at(&report, &["readback", "size"]), &Value::from(10usize));
    assert!(!text(&report, &["message"]).contains("romOverride"));
    assert!(text(&report, &["message"]).contains("should be running the card's own VBIOS"));
}

#[test]
fn a_removed_override_still_running_gets_the_restart_hint() {
    let (fake, _) = platform(RTX_5070, vec![]);
    fake.set_image(chain_of(RTX_5070, 4));
    load(&fake, &rom(RTX_5070, 4)).unwrap();
    unload(&fake).unwrap(); // the driver keeps the removed image until Windows restarts
    let report = verify(&fake, &none()).unwrap();
    assert_eq!(text(&report, &["state"]), "no-override");
    assert!(!text(&report, &["message"]).contains("the driver is running"));
    assert_eq!(
        text(&report, &["message"]),
        "No override is stored, so the driver should be running the card's own VBIOS. If one was just removed, \
         restart Windows first."
    );
    let report = verify_with(vec![("romOverride00", bin(b"r")), (VBIOS, sz("text"))], b"card image");
    assert_eq!(text(&report, &["state"]), "no-override");
    assert!(text(&report, &["message"]).contains("romOverride records are stored too"));
}

#[test]
fn two_different_reads_are_unstable() {
    let (fake, _) = platform(RTX_5070, vec![(VBIOS, bin(b"override"))]);
    fake.queue_image(Ok(b"override".to_vec()));
    fake.queue_image(Ok(b"overridf".to_vec()));
    let report = verify(&fake, &none()).unwrap();
    assert_eq!(
        (text(&report, &["state"]), at(&report, &["match"])),
        ("unstable", &Value::Null)
    );
    assert_eq!(at(&report, &["second_readback", "size"]), &Value::from(8usize));
    assert!(text(&report, &["message"]).contains("try again"));
}

#[test]
fn a_cut_readback_compares_the_prefix_and_says_so() {
    let big = big();
    let size = IMAGE_SIZE;
    let report = verify_with(vec![(VBIOS, bin(&big))], &big[..size]);
    assert_eq!(
        (
            text(&report, &["state"]),
            text(&report, &["compared"]),
            at(&report, &["readback", "truncated"])
        ),
        ("override-active", "prefix", &Value::Bool(true))
    );
    assert_eq!(
        text(&report, &["message"]),
        format!(
            "The first {size} bytes the driver is running match the stored override; NvAPI returns no more, so the \
             last {} bytes cannot be read back.",
            big.len() - size
        )
    );
    let mut first = vec![0xff];
    first.extend(&big[1..size]);
    let report = verify_with(vec![(VBIOS, bin(&big))], &first);
    assert_eq!(
        (
            text(&report, &["state"]),
            text(&report, &["compared"]),
            at(&report, &["first_difference"])
        ),
        ("override-not-active", "prefix", &Value::from(0usize))
    );
    let report = verify_with(vec![(VBIOS, bin(&big[..1000]))], &big[..size]); // shorter override: whole
    assert_eq!(
        (
            text(&report, &["state"]),
            text(&report, &["compared"]),
            at(&report, &["first_difference"])
        ),
        ("override-not-active", "full", &Value::from(1000usize))
    );
}

#[test]
fn selection_and_readback_errors() {
    let (fake, key) = platform(RTX_5070, vec![]);
    let by_pci = Selection {
        pci: Some("1".to_string()),
        ..Selection::default()
    };
    assert_error(verify(&fake, &by_pci), &["not a PCI location"]);
    fake.queue_image(Err(Error::new("this NVIDIA driver has no NvAPI_GPU_GetVbiosImage")));
    assert_error(verify(&fake, &none()), &["no NvAPI_GPU_GetVbiosImage"]);
    // The key was read and closed before NvAPI was asked.
    assert_eq!((key.closed(), fake.reads().len()), (1, 1));
}

#[test]
fn verify_report_layout() {
    let report = verify_with(vec![(VBIOS, bin(b"abc"))], b"abd");
    let keys: Vec<&str> = report.keys().collect();
    assert_eq!(
        keys,
        [
            "gpu",
            "readback",
            "configured",
            "match",
            "compared",
            "first_difference",
            "rom_override_records",
            "state",
            "message"
        ]
    );
    let (fake, _) = platform(RTX_5070, vec![(VBIOS, bin(b"abc"))]);
    fake.queue_image(Ok(b"abc".to_vec()));
    fake.queue_image(Ok(b"abd".to_vec()));
    let report = verify(&fake, &none()).unwrap();
    let keys: Vec<&str> = report.keys().collect();
    assert_eq!(
        keys,
        [
            "gpu",
            "readback",
            "configured",
            "match",
            "compared",
            "first_difference",
            "rom_override_records",
            "second_readback",
            "state",
            "message"
        ]
    );
}
