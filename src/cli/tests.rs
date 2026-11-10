//! The tests of tests/test_windows_loader.py and tests/test_windows_loader_verify.py that go through cli.main
//! (CliTests, VerifyCliTests and the command line parts of RawFirmwareTests), with the same inputs and expected
//! texts, plus the argparse behaviour, help layout and helpers of this module checked against Python's output.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::devices::{Decoded, REG_DWORD, REG_QWORD};
use crate::operations::{self, ABSENT, BACKUP, FIRMWARE, MAX_SAVED, RESTART_HINT, VBIOS};
use crate::testing::{binary as bin, chain, dword, platform, raw, rom, sz, unrelated, FakePlatform, ImageOptions};
use crate::testing::{FakeKey, RTX_5070};

/// A directory for the test's files, removed afterwards (tempfile.TemporaryDirectory).
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let number = COUNT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("vbios-loader-cli-test-{}-{number}", std::process::id()));
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempDir(path)
    }

    /// Writes `data` to `name` in the directory; returns the path as text.
    fn file(&self, name: &str, data: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, data).expect("a temporary file");
        self.path(name)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// CliTests.run_cli: (exit code, stdout, stderr) at 80 columns.
fn run_cli(argv: &[&str], fake: &Arc<FakePlatform>) -> (i32, String, String) {
    run_at(argv, Some(fake.clone()), 80)
}

fn run_at(argv: &[&str], platform: Option<Arc<FakePlatform>>, columns: i128) -> (i32, String, String) {
    let args: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let platform = platform.map(|fake| fake as Arc<dyn Platform>);
    let code = run_with_columns(&args, platform, &mut out, &mut err, &|| columns);
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("UTF-8 output");
    (code, text(out), text(err))
}

fn shared((fake, key): (FakePlatform, FakeKey)) -> (Arc<FakePlatform>, FakeKey) {
    (Arc::new(fake), key)
}

fn empty_platform() -> Arc<FakePlatform> {
    Arc::new(FakePlatform::new(
        Vec::new(),
        Vec::<(String, FakeKey)>::new(),
        true,
        Vec::new(),
    ))
}

fn chain_of(device: u16) -> Vec<u8> {
    chain(device, 4, ImageOptions::default())
}

/// ops.load(fake, data) with the only GPU.
fn load_directly(fake: &FakePlatform, data: &[u8]) {
    operations::load(fake, data, &Selection::default(), false).expect("load");
}

fn values(key: &FakeKey) -> Vec<(String, Decoded)> {
    let mut values = key.values();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

fn unrelated_sorted() -> Vec<(String, Decoded)> {
    let mut values = unrelated();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

// json.loads for the tests: a small parser of what json.dumps writes.
struct Json<'a> {
    text: &'a [u8],
    at: usize,
}

impl Json<'_> {
    fn parse(text: &str) -> Value {
        let mut parser = Json {
            text: text.as_bytes(),
            at: 0,
        };
        let value = parser.value();
        parser.space();
        assert_eq!(
            parser.at,
            parser.text.len(),
            "trailing text after the JSON value: {text:?}"
        );
        value
    }

    fn space(&mut self) {
        while self.text.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn eat(&mut self, expected: &str) {
        assert!(
            self.text[self.at..].starts_with(expected.as_bytes()),
            "expected {expected:?} at {}",
            self.at
        );
        self.at += expected.len();
    }

    fn value(&mut self) -> Value {
        self.space();
        match self.text[self.at] {
            b'{' => {
                self.eat("{");
                let mut map = Map::new();
                self.space();
                if self.text[self.at] == b'}' {
                    self.eat("}");
                    return Value::Object(map);
                }
                loop {
                    self.space();
                    let key = self.string();
                    self.space();
                    self.eat(":");
                    let value = self.value();
                    map.insert(&key, value);
                    self.space();
                    if self.text[self.at] == b',' {
                        self.eat(",");
                    } else {
                        self.eat("}");
                        return Value::Object(map);
                    }
                }
            }
            b'[' => {
                self.eat("[");
                let mut items = Vec::new();
                self.space();
                if self.text[self.at] == b']' {
                    self.eat("]");
                    return Value::Array(items);
                }
                loop {
                    items.push(self.value());
                    self.space();
                    if self.text[self.at] == b',' {
                        self.eat(",");
                    } else {
                        self.eat("]");
                        return Value::Array(items);
                    }
                }
            }
            b'"' => Value::Str(self.string()),
            b't' => {
                self.eat("true");
                Value::Bool(true)
            }
            b'f' => {
                self.eat("false");
                Value::Bool(false)
            }
            b'n' => {
                self.eat("null");
                Value::Null
            }
            _ => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(|c| *c == b'-' || c.is_ascii_digit()) {
                    self.at += 1;
                }
                let number = std::str::from_utf8(&self.text[start..self.at]).expect("ASCII");
                Value::Int(number.parse().expect("an integer"))
            }
        }
    }

    fn string(&mut self) -> String {
        self.eat("\"");
        let mut units: Vec<u16> = Vec::new();
        loop {
            let c = self.text[self.at];
            self.at += 1;
            match c {
                b'"' => return String::from_utf16(&units).expect("valid UTF-16"),
                b'\\' => {
                    let escaped = self.text[self.at];
                    self.at += 1;
                    let unit = match escaped {
                        b'n' => u16::from(b'\n'),
                        b'r' => u16::from(b'\r'),
                        b't' => u16::from(b'\t'),
                        b'b' => 8,
                        b'f' => 12,
                        b'u' => {
                            let hex = std::str::from_utf8(&self.text[self.at..self.at + 4]).expect("ASCII");
                            self.at += 4;
                            u16::from_str_radix(hex, 16).expect("hex")
                        }
                        other => u16::from(other),
                    };
                    units.push(unit);
                }
                other => {
                    assert!(other.is_ascii(), "json.dumps writes ASCII only");
                    units.push(u16::from(other));
                }
            }
        }
    }
}

/// `value[a][b]...` of parsed JSON.
fn at<'a>(value: &'a Value, path: &[&str]) -> &'a Value {
    path.iter().fold(value, |value, name| {
        value.get(name).unwrap_or_else(|| panic!("no {name:?} in {value:?}"))
    })
}

fn first(value: &Value) -> &Value {
    &value.as_array().expect("a list")[0]
}

#[test]
fn status_json_without_gpus_is_an_empty_list() {
    let fake = empty_platform();
    let (code, out, _) = run_cli(&["status", "--json"], &fake);
    assert_eq!((code, out.as_str()), (0, "[]\n"));
    let (code, out, _) = run_cli(&["status"], &fake);
    assert_eq!((code, out.as_str()), (0, "No NVIDIA GPU found.\n"));
}

#[test]
fn status_reports_the_stored_override() {
    let (fake, key) = shared(platform(RTX_5070, vec![("romOverride02", bin(b"x"))]));
    load_directly(&fake, &rom(RTX_5070, 4));
    key.insert("romOverride02", bin(b"x"));
    let (code, out, _) = run_cli(&["status", "--json"], &fake);
    let report = Json::parse(&out);
    let entry = first(&report);
    assert_eq!(code, 0);
    assert_eq!(
        (
            at(entry, &["location"]).as_str(),
            at(entry, &["pci_id"]).as_str(),
            at(entry, &["nvidia_driver_version"]).as_str()
        ),
        (Some("01:00.0"), Some("10de:2c09"), Some("616.56"))
    );
    let size = i64::try_from(chain_of(RTX_5070).len()).unwrap();
    assert_eq!(at(entry, &["override", "vbios", "size"]).as_i64(), Some(size));
    assert!(at(entry, &["override", "enable_gpu_firmware"]).is_null());
    assert_eq!(
        at(entry, &["override", "saved_previous_enable_gpu_firmware"]).as_str(),
        Some("absent")
    );
    assert_eq!(
        at(entry, &["override", "gsp_forced_by_old_loader"]),
        &Value::Bool(false)
    );
    assert_eq!(
        at(entry, &["override", "rom_override_records"]),
        &Value::Array(vec!["romOverride02".into()])
    );
    let out = run_cli(&["status"], &fake).1;
    assert!(out.contains("  EnableGpuFirmware: not set (saved previous value: absent)\n"));
    assert!(out.contains("romOverride records: 1"));
    assert!(!out.contains("GSP firmware mode"));
}

#[test]
fn status_flags_gpus_left_in_gsp_firmware_mode_by_0_3_x() {
    let line = "  GSP firmware mode: on, set by loader 0.3.2 or older; load the file again with this version (or \
                unload it) and restart Windows.";
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
        let (fake, _) = shared(platform(RTX_5070, values.clone()));
        let (code, out, _) = run_cli(&["status", "--json"], &fake);
        assert_eq!(code, 0);
        let report = Json::parse(&out);
        assert_eq!(
            at(first(&report), &["override", "gsp_forced_by_old_loader"]),
            &Value::Bool(flagged),
            "{values:?}"
        );
        let out = run_cli(&["status"], &fake).1;
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.contains(&line), flagged, "{values:?}");
        if flagged {
            // Right after the EnableGpuFirmware line.
            let index = lines.iter().position(|l| *l == line).unwrap();
            assert!(lines[index - 1].starts_with("  EnableGpuFirmware: 0x11 (saved "));
        }
    }
}

#[test]
fn load_and_unload_commands() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let hint = format!("\n{RESTART_HINT}\n");
    assert_eq!(
        RESTART_HINT,
        "Restart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. With it on, \
         the change waits for the next restart, and nothing has to be repeated."
    );
    let directory = TempDir::new();
    let path = directory.file("edited.rom", &rom(RTX_5070, 4));
    let (code, out, _) = run_cli(&["load", &path, "--pci", "01:00.0", "--dry-run"], &fake);
    assert_eq!(code, 0);
    assert!(out.ends_with("\nDry run: nothing was written.\n"));
    assert!(!out.contains("Restart"));
    let (code, out, _) = run_cli(&["load", &path, "--gpu", "0", "--json"], &fake);
    let report = Json::parse(&out);
    assert_eq!(
        (code, at(&report, &["ok"]), at(&report, &["next_step"]).as_str()),
        (0, &Value::Bool(true), Some(RESTART_HINT))
    );
    assert!(report.get("restart").is_none());
    let (code, out, _) = run_cli(&["load", &path], &fake);
    assert_eq!(code, 0);
    assert!(out.ends_with(&hint));
    let (code, _, err) = run_cli(&["load", &directory.path("missing.rom")], &fake);
    assert_eq!(code, 1);
    assert!(err.contains("cannot read"));
    // --no-restart is still accepted, so older scripts and instructions keep working; it changes nothing.
    let mut reports = Vec::new();
    for extra in [&[][..], &["--no-restart"][..]] {
        let (other, _) = shared(platform(RTX_5070, vec![]));
        let mut argv = vec!["load", path.as_str(), "--json"];
        argv.extend(extra);
        let (code, out, _) = run_cli(&argv, &other);
        reports.push((code, Json::parse(&out)));
    }
    assert_eq!(reports[0], reports[1]);
    drop(directory);
    let (code, out, _) = run_cli(&["unload", "--no-restart"], &fake);
    assert_eq!(code, 0);
    assert!(out.ends_with(&hint));
    assert_eq!(values(&key), unrelated_sorted());
    let (code, out, _) = run_cli(&["load", "--help"], &fake);
    assert_eq!(code, 0);
    let words: Vec<&str> = out.split_whitespace().collect();
    assert!(words
        .join(" ")
        .contains("--no-restart no effect, kept for compatibility: the loader no longer restarts the GPU"));
}

#[test]
fn missing_files_are_named_with_the_system_error_text() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let directory = TempDir::new();
    let missing = directory.path("missing.rom");
    let (code, out, err) = run_cli(&["load", &missing, "--json"], &fake);
    let message = format!("cannot read {missing}: No such file or directory");
    let report = Json::parse(&out);
    assert_eq!(code, 1);
    assert_eq!(at(&report, &["ok"]), &Value::Bool(false));
    assert_eq!(at(&report, &["error"]).as_str(), Some(message.as_str()));
    assert_eq!(err, format!("vbios-loader: {message}\n"));
    assert_eq!(key.calls(), vec![]);
}

/// A folder given as the file: Windows does not open a folder as a file (ERROR_ACCESS_DENIED), which Python's
/// open() showed as "Permission denied".
#[test]
#[cfg(windows)]
fn a_folder_is_refused_as_python_refused_it_on_windows() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let directory = TempDir::new();
    let folder = directory.path("folder");
    std::fs::create_dir(&folder).expect("a temporary folder");
    let (code, _, err) = run_cli(&["load", &folder], &fake);
    assert_eq!(code, 1);
    assert_eq!(err, format!("vbios-loader: cannot read {folder}: Permission denied\n"));
    assert_eq!(key.calls(), vec![]);
}

#[test]
#[cfg(not(windows))]
fn other_systems_fail_cleanly_without_loading_windows_code() {
    let (code, out, err) = run_at(&["status", "--json"], None, 80);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("runs on Windows only"));
    assert_eq!(at(&Json::parse(&out), &["ok"]), &Value::Bool(false));
    assert_eq!(err, "vbios-loader: this tool runs on Windows only\n");
}

#[test]
fn status_shows_the_type_and_the_saved_value() {
    let qword = 0x11u64.to_le_bytes();
    let (fake, _) = shared(platform(RTX_5070, vec![(FIRMWARE, raw(REG_QWORD, &qword))]));
    load_directly(&fake, &rom(RTX_5070, 4));
    let out = run_cli(&["status"], &fake).1;
    assert!(out.contains("EnableGpuFirmware: unexpected REG_QWORD 0x11 (saved previous value: REG_QWORD 0x11)"));
}

#[test]
fn dry_runs_show_the_steps() {
    let qword = 0x11u64.to_le_bytes();
    let (fake, key) = shared(platform(RTX_5070, vec![(FIRMWARE, raw(REG_QWORD, &qword))]));
    fake.set_admin(false);
    let directory = TempDir::new();
    let path = directory.file("edited.rom", &rom(RTX_5070, 4));
    let (code, out, _) = run_cli(&["load", &path, "--dry-run"], &fake);
    assert_eq!(code, 0);
    assert!(out.contains(
        "  set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(b):11,00,00,00,00,00,00,00\n  set vbios = REG_BINARY "
    ));
    assert!(!out.contains("set EnableGpuFirmware"));
    assert!(out.ends_with("\nDry run: nothing was written.\n"));
    assert_eq!(key.calls(), vec![]);

    let (fake, _) = platform(
        RTX_5070,
        vec![
            (VBIOS, bin(b"x")),
            (FIRMWARE, dword(0x11)),
            (BACKUP, sz("hex(1):31,00,00,00,37,00")),
        ],
    );
    fake.set_admin(false);
    let report = operations::unload(&fake, &Selection::default(), true).unwrap();
    let step = &report.get("steps").and_then(Value::as_array).unwrap()[0];
    assert_eq!(step_text(step), "set EnableGpuFirmware = REG_SZ '1\\x007' (6 bytes)");

    // A long saved value is shortened in the step lines; the JSON steps keep all of it.
    let (fake, _) = platform(RTX_5070, vec![(FIRMWARE, bin(&vec![0; MAX_SAVED]))]);
    fake.set_admin(false);
    let report = operations::load(&fake, &rom(RTX_5070, 4), &Selection::default(), true).unwrap();
    let step = &report.get("steps").and_then(Value::as_array).unwrap()[0];
    assert_eq!(
        step_text(step),
        format!(
            "set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(3):{}0... (12294 characters)",
            "00,".repeat(24)
        )
    );
}

#[test]
fn step_text_forms() {
    let step = |entries: &[(&str, Value)]| {
        let mut map = Map::new();
        for (name, value) in entries {
            map.insert(name, value.clone());
        }
        Value::Object(map)
    };
    assert_eq!(
        step_text(&step(&[("op", "delete".into()), ("name", "vbios".into())])),
        "delete vbios"
    );
    assert_eq!(
        step_text(&step(&[
            ("op", "set".into()),
            ("name", "vbios".into()),
            ("type", "REG_BINARY".into()),
            ("size", 3usize.into()),
            ("sha256", "ab".into())
        ])),
        "set vbios = REG_BINARY 3 bytes, sha256 ab"
    );
    // Exactly 100 characters stay; the length counts characters, not bytes.
    let hundred = "é".repeat(100);
    let set = |value: &str| {
        step_text(&step(&[
            ("op", "set".into()),
            ("name", "x".into()),
            ("type", "REG_SZ".into()),
            ("value", value.into()),
        ]))
    };
    assert_eq!(set(&hundred), format!("set x = REG_SZ {hundred}"));
    assert_eq!(
        set(&format!("{hundred}a")),
        format!("set x = REG_SZ {}... (101 characters)", "é".repeat(80))
    );
    assert_eq!(
        step_text(&step(&[
            ("op", "set".into()),
            ("name", "x".into()),
            ("type", "REG_NONE".into())
        ])),
        "set x = REG_NONE "
    );
}

/// BIG of test_windows_loader_verify.py: longer than what NvAPI returns.
fn big() -> Vec<u8> {
    (0..4001).flat_map(|_| 0..=255u8).collect()
}

#[test]
fn verify_states_and_exit_codes() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    fake.set_admin(false);
    fake.set_image(b"card image".to_vec());
    let (code, out, _) = run_cli(&["verify"], &fake);
    assert_eq!(code, 0);
    assert!(out.contains("  running VBIOS (NvAPI): 10 bytes, sha256 "));
    assert!(out.contains("  stored override: none"));
    assert!(out.ends_with("If one was just removed, restart Windows first.\n"));
    key.insert(VBIOS, bin(b"override"));
    let (code, out, _) = run_cli(&["verify", "--gpu", "0", "--json"], &fake);
    let report = Json::parse(&out);
    assert_eq!(
        (
            code,
            at(&report, &["ok"]),
            at(&report, &["state"]).as_str(),
            at(&report, &["first_difference"]).as_i64()
        ),
        (4, &Value::Bool(false), Some("override-not-active"), Some(0))
    );
    let big = big();
    fake.set_image(big[..IMAGE_SIZE].to_vec());
    key.insert(VBIOS, bin(&big));
    let (code, out, _) = run_cli(&["verify", "--pci", "01:00.0"], &fake);
    assert_eq!(code, 0);
    assert!(out.contains(&format!("{IMAGE_SIZE} bytes, sha256 ")));
    assert!(out.contains(&format!("(NvAPI returns at most {IMAGE_SIZE} bytes)")));
    assert!(out.contains(&format!("  stored override: {} bytes", big.len())));
    assert!(!out.contains("verified byte by byte"));
    fake.queue_image(Ok(b"override".to_vec()));
    fake.queue_image(Ok(b"overridf".to_vec()));
    key.insert(VBIOS, bin(b"override"));
    let (code, out, _) = run_cli(&["verify"], &fake);
    assert_eq!(code, 4);
    assert!(out.contains("  running VBIOS (NvAPI): 8 bytes, sha256 "));
    assert!(out.contains("  second read: 8 bytes, sha256 "));
}

#[test]
fn verify_errors_and_options() {
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    fake.queue_image(Err(Error::new("cannot load nvapi64.dll")));
    let (code, out, err) = run_cli(&["verify", "--json"], &fake);
    let mut expected = Map::new();
    expected.insert("ok", false);
    expected.insert("error", "cannot load nvapi64.dll");
    assert_eq!((code, Json::parse(&out)), (1, Value::Object(expected)));
    assert!(err.contains("vbios-loader: cannot load nvapi64.dll"));
    let (code, out, _) = run_cli(&["verify", "--dry-run"], &fake);
    assert_eq!((code, out.as_str()), (2, ""));
}

/// Help, version and usage errors as Python 3.12's argparse printed them for cli.py (with version 0.5.0).
#[rustfmt::skip]
const HELP: [(&[&str], i128, i32, &str, &str); 5] = [
    (
        &["-h"],
        80,
        0,
        r#"usage: vbios-loader [-h] [--version] {status,load,unload,verify} ...

vbios-loader: load a modified VBIOS on Windows through the NVIDIA driver's
registry override.

positional arguments:
  {status,load,unload,verify}
    status              list NVIDIA GPUs and their override state
    load                store a VBIOS override for one GPU; it applies after
                        restarting Windows
    unload              remove the override and restore the previous firmware
                        setting; it applies after restarting Windows
    verify              compare the VBIOS the driver is running with the
                        stored override

options:
  -h, --help            show this help message and exit
  --version             show program's version number and exit
"#,
        r#""#,
    ),
    (
        &["load", "--help"],
        40,
        0,
        r#"usage: vbios-loader load [-h]
                         [--gpu INDEX]
                         [--pci BUS:DEV.FN]
                         [--dry-run]
                         [--no-restart]
                         [--json]
                         payload

positional arguments:
  payload         VBIOS file (see the
                  README for which
                  file to use)

options:
  -h, --help      show this help
                  message and exit
  --gpu INDEX     GPU index as shown
                  by status
  --pci BUS:DEV.FN
                  PCI location as
                  shown by status, for
                  example 01:00.0
  --dry-run       show the registry
                  changes without
                  making them
  --no-restart    no effect, kept for
                  compatibility: the
                  loader no longer
                  restarts the GPU
  --json          machine-readable
                  output
"#,
        r#""#,
    ),
    (
        &["--version"],
        10,
        0,
        r#"vbios-
loader
0.5.0
"#,
        r#""#,
    ),
    (
        &["unload", "--gpu", "1", "--pci"],
        80,
        2,
        r#""#,
        r#"usage: vbios-loader unload [-h] [--gpu INDEX] [--pci BUS:DEV.FN] [--dry-run]
                           [--no-restart] [--json]
vbios-loader unload: error: argument --pci: expected one argument
"#,
    ),
    (
        &["verify", "-h"],
        25,
        0,
        r#"usage: vbios-loader verify
       [-h]
       [--gpu INDEX]
       [--pci BUS:DEV.FN]
       [--json]

options:
  -h, --help
    show this help
    message and exit
  --gpu INDEX
    GPU index as shown
    by status
  --pci BUS:DEV.FN
    PCI location as
    shown by status,
    for example 01:00.0
  --json
    machine-readable
    output
"#,
        r#""#,
    ),
];

#[test]
fn help_version_and_usage_errors_are_argparse_texts() {
    let fake = empty_platform();
    for (argv, columns, code, out, err) in HELP {
        let result = run_at(argv, Some(fake.clone()), columns);
        assert_eq!(
            result,
            (code, out.to_string(), err.to_string()),
            "{argv:?} at {columns} columns"
        );
    }
}

/// (exit code, stdout, stderr) of a command line that argparse refuses or answers itself.
fn refused(argv: &[&str]) -> (i32, String) {
    let (code, out, err) = run_at(argv, Some(empty_platform()), 80);
    assert_eq!(out, "");
    (code, err.lines().last().unwrap_or_default().to_string())
}

#[test]
fn argparse_errors() {
    let cases: [(&[&str], &str); 16] = [
        (
            &[],
            "vbios-loader: error: the following arguments are required: command",
        ),
        (
            &["0"],
            "vbios-loader: error: argument command: invalid choice: '0' (choose from status, load, unload, verify)",
        ),
        (
            &["--gpu", "0", "load"],
            "vbios-loader: error: argument command: invalid choice: '0' (choose from status, load, unload, verify)",
        ),
        (
            &["-=x"],
            "vbios-loader: error: ambiguous option: -=x could match -h, --help, --version",
        ),
        (
            &["--=x"],
            "vbios-loader: error: ambiguous option: --=x could match --help, --version",
        ),
        (
            &["-h=x"],
            "vbios-loader: error: argument -h/--help: ignored explicit argument 'x'",
        ),
        (
            &["status", "--json=1"],
            "vbios-loader status: error: argument --json: ignored explicit argument '1'",
        ),
        (
            &["load"],
            "vbios-loader load: error: the following arguments are required: payload",
        ),
        (&["load", "a", "b"], "vbios-loader: error: unrecognized arguments: b"),
        (
            &["verify", "--dry-run"],
            "vbios-loader: error: unrecognized arguments: --dry-run",
        ),
        (
            &["load", "--gpu", "x", "a"],
            "vbios-loader load: error: argument --gpu: invalid int value: 'x'",
        ),
        (
            &["load", "a", "--gpu", "-x"],
            "vbios-loader load: error: argument --gpu: expected one argument",
        ),
        (
            &["load", "--gpu=", "a"],
            "vbios-loader load: error: argument --gpu: invalid int value: ''",
        ),
        (
            &["status", "--", "--json"],
            "vbios-loader: error: unrecognized arguments: -- --json",
        ),
        (
            &["-x", "status", "--foo"],
            "vbios-loader: error: unrecognized arguments: -x --foo",
        ),
        (
            &["load", "--", "--gpu", "1", "x"],
            "vbios-loader: error: unrecognized arguments: 1 x",
        ),
    ];
    for (argv, message) in cases {
        assert_eq!(refused(argv), (2, message.to_string()), "{argv:?}");
    }
}

#[test]
fn argparse_accepts_abbreviations_values_and_negative_numbers() {
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    let directory = TempDir::new();
    let path = directory.file("edited.rom", &rom(RTX_5070, 4));
    // Unique prefixes, --opt=value, a value that looks like a negative number, Unicode digits for int().
    for argv in [
        vec!["load", "--g", "0", "--dr", "--j", path.as_str()],
        vec!["load", path.as_str(), "--gpu=\u{660}", "--dry", "--js"],
        vec!["load", "--p=01:00.0", "--d", "--no", "--json", "--", path.as_str()],
        vec!["load", "--gpu", " 0 ", "--dry-run", "--json", path.as_str()],
    ] {
        let (code, out, err) = run_cli(&argv, &fake);
        assert_eq!((code, err.as_str()), (0, ""), "{argv:?}");
        assert_eq!(at(&Json::parse(&out), &["dry_run"]), &Value::Bool(true), "{argv:?}");
    }
    let (code, out, err) = run_cli(&["unload", "--gpu", "-1", "--json"], &fake);
    assert_eq!(code, 1);
    assert_eq!(
        at(&Json::parse(&out), &["error"]).as_str(),
        Some("no NVIDIA GPU matches that selection; run status to list them")
    );
    assert_eq!(
        err,
        "vbios-loader: no NVIDIA GPU matches that selection; run status to list them\n"
    );
    let (code, _, err) = run_cli(&["unload", "--gpu", "0", "--pci", "01:00.0"], &fake);
    assert_eq!(
        (code, err.as_str()),
        (1, "vbios-loader: use either --gpu or --pci, not both\n")
    );
    // -h wins wherever it is parsed first; an error before it wins.
    assert_eq!(run_cli(&["load", "-hx"], &fake).0, 0);
    assert_eq!(run_cli(&["load", "--gpu", "x", "-h"], &fake).0, 2);
    assert_eq!(
        run_cli(&["--version", "bogus"], &fake),
        (0, "vbios-loader 0.5.0\n".into(), String::new())
    );
}

#[test]
fn paths_print_as_pathlib_does() {
    // (text, str(PurePosixPath(text)), str(PureWindowsPath(text))) from Python 3.12.
    let cases = [
        ("", ".", "."),
        (".", ".", "."),
        ("a//b/./c/", "a/b/c", "a\\b\\c"),
        ("//x/y", "//x/y", "\\\\x\\y\\"),
        ("///x", "/x", "\\\\\\x"),
        ("/", "/", "\\"),
        ("./a", "a", "a"),
        ("a/..", "a/..", "a\\.."),
        ("C:/x\\y", "C:/x\\y", "C:\\x\\y"),
        ("c:x", "c:x", "c:x"),
        ("./c:x", "c:x", ".\\c:x"),
        ("//server/share", "//server/share", "\\\\server\\share\\"),
        ("//server/share/x", "//server/share/x", "\\\\server\\share\\x"),
        ("\\\\?\\C:\\a\\.\\b", "\\\\?\\C:\\a\\.\\b", "\\\\?\\C:\\a\\b"),
        ("//?/UNC/srv/sh", "//?/UNC/srv/sh", "\\\\?\\UNC\\srv\\sh\\"),
        ("\\\\.\\dev", "\\\\.\\dev", "\\\\.\\dev"),
        ("\\x", "\\x", "\\x"),
        ("x:", "x:", "x:"),
        ("//", "//", "\\\\"),
        ("\\\\a", "\\\\a", "\\\\a"),
    ];
    for (text, posix, windows) in cases {
        assert_eq!(path_text(text, false), posix, "{text:?}");
        assert_eq!(path_text(text, true), windows, "{text:?}");
    }
}

#[test]
fn errors_read_as_python_strerror() {
    // _dosmaperr and the C runtime texts: what Python's open() shows on Windows.
    let windows = |code: i32| crt_strerror(crt_errno(code));
    assert_eq!(windows(2), "No such file or directory");
    assert_eq!(windows(3), "No such file or directory");
    assert_eq!(windows(5), "Permission denied");
    assert_eq!(windows(32), "Permission denied"); // a sharing violation
    assert_eq!(windows(123), "Invalid argument"); // an invalid file name
    assert_eq!(windows(206), "No such file or directory");
    assert_eq!(windows(112), "No space left on device");
    assert_eq!(windows(1), "Invalid argument");
    #[cfg(not(windows))]
    {
        let error = std::fs::read("/nonexistent/vbios-loader-test").unwrap_err();
        assert_eq!(os_error_text(&error), "No such file or directory");
    }
}

#[test]
fn terminal_width_follows_shutil() {
    assert_eq!(terminal_columns(None, || None), 80);
    assert_eq!(terminal_columns(Some("100"), || Some(50)), 100);
    assert_eq!(terminal_columns(Some(" 1_00\n"), || None), 100);
    assert_eq!(terminal_columns(Some("0"), || Some(120)), 120);
    assert_eq!(terminal_columns(Some("-3"), || None), 80);
    assert_eq!(terminal_columns(Some("wide"), || Some(0)), 80);
    assert_eq!(terminal_columns(Some("\u{661}\u{662}"), || None), 12);
}

#[test]
fn newlines_are_translated_for_windows() {
    let mut out = Newlines {
        inner: Vec::new(),
        newline: "\r\n",
    };
    write!(out, "a\nb\n\nc").unwrap();
    assert_eq!(out.inner, b"a\r\nb\r\n\r\nc");
    let mut out = Newlines {
        inner: Vec::new(),
        newline: "\n",
    };
    write!(out, "a\nb").unwrap();
    assert_eq!(out.inner, b"a\nb");
}

#[test]
fn a_panic_is_exit_code_70() {
    assert_eq!(guarded(|| 4), 4);
    let fake = empty_platform();
    let gpu = crate::testing::adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656"));
    fake.set_adapters(vec![gpu]);
    fake.panic_on_open(Some("boom"));
    assert_eq!(guarded(|| run_cli(&["status"], &fake).0), 70);
    assert_eq!(panic_text(&"text"), "text");
    assert_eq!(panic_text(&String::from("owned")), "owned");
    assert_eq!(panic_text(&3), "Box<dyn Any>");
}

#[test]
fn reports_without_a_payload_or_steps() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let (code, out, _) = run_cli(&["unload"], &fake);
    assert_eq!(code, 0);
    assert!(
        out.ends_with("\nRegistry changes:\nNo override is stored for this GPU.\n"),
        "{out}"
    );
    assert_eq!(key.calls(), vec![]);
}
