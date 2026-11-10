//! The ControllerTests of tests/test_windows_loader_gui.py, with the same inputs and expected texts. What the
//! Python tests set directly (`c.busy = True`, `fake.admin = ...`, a replaced open_key or vbios_image) goes through
//! `start`, the FakePlatform setters and its queued results here; the Tk test keeps its controller assertions.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::devices::{Adapter, Decoded, REG_QWORD, REG_SZ};
use crate::error::Error;
use crate::operations::{ABSENT, BACKUP, FIRMWARE, VBIOS};
use crate::testing::{
    adapter, binary as bin, chain, dword, platform, platform_with_version, raw, rom, sz, unrelated, FakeKey,
    FakePlatform, ImageOptions, RTX_5070, RTX_5070_TI,
};

/// A directory for the test's files, removed afterwards (the setUp of ControllerTests).
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let number = COUNT.fetch_add(1, Ordering::Relaxed);
        let name = format!("vbios-loader-controller-test-{}-{number}", std::process::id());
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&path).expect("a temporary directory");
        TempDir(path)
    }

    /// ControllerTests.file: writes the data and returns the path.
    fn file(&self, data: &[u8]) -> String {
        let path = self.0.join("edited.rom");
        std::fs::write(&path, data).expect("a temporary file");
        path.to_string_lossy().into_owned()
    }

    fn missing(&self) -> String {
        self.0.join("missing.rom").to_string_lossy().into_owned()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn en(key: &str) -> &'static str {
    text(Language::En, key)
}

/// ControllerTests.controller: a controller whose first refresh has no message lines.
fn controller(fake: &Arc<FakePlatform>, language: Language) -> Controller {
    let mut c = Controller::new(fake.clone(), language, Theme::Auto);
    assert_eq!(c.refresh(), Vec::<String>::new());
    c
}

/// controller.run(action), as the window does it: start, the job (here on this thread), finish.
fn run(c: &mut Controller, action: Action) -> (bool, Vec<String>) {
    let job = c.start(action);
    assert_eq!(job.action(), action);
    let result = job.run();
    c.finish();
    result
}

fn shared((fake, key): (FakePlatform, FakeKey)) -> (Arc<FakePlatform>, FakeKey) {
    (Arc::new(fake), key)
}

fn new_platform(adapters: Vec<Adapter>, keys: Vec<(String, FakeKey)>) -> Arc<FakePlatform> {
    Arc::new(FakePlatform::new(adapters, keys, true, Vec::new()))
}

fn chain_len() -> usize {
    chain(RTX_5070, 4, ImageOptions::default()).len()
}

fn with_unrelated(values: Vec<(&str, Decoded)>) -> Vec<(String, Decoded)> {
    let mut all = unrelated();
    all.extend(values.into_iter().map(|(name, value)| (name.to_string(), value)));
    all
}

fn sorted(mut values: Vec<(String, Decoded)>) -> Vec<(String, Decoded)> {
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

fn names(key: &FakeKey) -> BTreeSet<String> {
    key.values().into_iter().map(|(name, _)| name).collect()
}

/// The location of the selected GPU, from its row (c.gpu['location']).
fn selected_location(c: &Controller) -> Option<String> {
    let index = c.selected_index()?;
    c.rows()
        .into_iter()
        .find(|row| row.index == index)
        .map(|row| row.location)
}

#[test]
fn rows_show_the_override_state_from_status() {
    let gpus = vec![
        adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
        adapter(1, RTX_5070, Some("02:00.0"), Some("32.0.16.1656")),
        adapter(2, RTX_5070, Some("03:00.0"), Some("32.0.15.6094")),
        adapter(3, RTX_5070, Some("04:00.0"), Some("32.0.16.1656")),
    ];
    let keys = vec![
        (gpus[0].instance_id.clone(), FakeKey::new(unrelated())),
        (
            gpus[1].instance_id.clone(),
            FakeKey::new([(VBIOS, bin(b"mvolt")), ("romOverride00", bin(b"r"))]),
        ),
        (
            gpus[2].instance_id.clone(),
            FakeKey::new([(VBIOS, bin(b"ours")), (BACKUP, sz(ABSENT))]),
        ),
    ];
    let fake = new_platform(gpus.clone(), keys);
    // Python's open_key raised OSError(5, 'Access is denied') for the GPU without a key.
    fake.fail_open(&gpus[3].instance_id, Error::new("[Errno 5] Access is denied"));
    let rows = controller(&fake, Language::En).rows();
    assert_eq!(
        rows[0],
        Row {
            index: 0,
            name: "GPU 0".into(),
            pci_id: "10de:2c09".into(),
            location: "01:00.0".into(),
            driver: "616.56".into(),
            override_text: "none".into(),
        }
    );
    assert_eq!(
        rows[1].override_text,
        "other tool's override: 5 bytes; romOverride records (mVolt+): 1"
    );
    assert_eq!(rows[2].driver, "560.94 (too old, 616.56 or newer needed)");
    assert!(rows[2].override_text.starts_with("loaded: 4 bytes, sha256 "));
    assert_eq!(rows[2].override_text.len(), "loaded: 4 bytes, sha256 ...".len() + 12);
    assert_eq!(rows[3].override_text, "cannot read the driver key");
}

#[test]
fn selection_and_validation_use_the_dry_run() {
    let gpus = vec![
        adapter(0, RTX_5070, Some("01:00.0"), Some("32.0.16.1656")),
        adapter(1, RTX_5070_TI, Some("02:00.0"), Some("32.0.16.1656")),
    ];
    let keys: Vec<(String, FakeKey)> = gpus
        .iter()
        .map(|gpu| (gpu.instance_id.clone(), FakeKey::new(unrelated())))
        .collect();
    let fake = new_platform(gpus, keys.clone());
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    assert_eq!(c.selected_index(), Some(0)); // the first GPU is selected
    assert_eq!(c.validation_text(), (false, "Choose a VBIOS file.".to_string()));
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    let (ok, shown) = c.validation_text();
    assert!(ok);
    assert!(shown.contains(&format!(
        "OK for GPU 0: blackwell-gb20x, {} bytes to store, sha256 ",
        chain_len()
    )));
    c.select(1);
    let (ok, shown) = c.validation_text();
    assert!(!ok);
    assert!(
        shown.starts_with("Refused: payload refused for GPU 1 (10de:2c05): "),
        "{shown}"
    );
    assert!(shown.contains("not 10de:2c05"), "{shown}");
    assert!(!c.can_load());
    c.refresh();
    assert_eq!(c.selected_index(), Some(1)); // kept across a refresh
    c.choose_file(&files.missing());
    assert!(c.validation_text().1.starts_with("Refused: Cannot read "));
    assert_eq!(keys.iter().filter(|(_, key)| !key.calls().is_empty()).count(), 0);
    assert_eq!(fake.writable_opens(), 0);
    let empty = new_platform(Vec::new(), Vec::new());
    assert_eq!(
        controller(&empty, Language::En).validation_text(),
        (false, "No NVIDIA GPU found.".to_string())
    );
}

#[test]
fn old_driver_is_listed_and_refused_with_the_existing_message() {
    let (fake, _) = shared(platform_with_version(RTX_5070, vec![], Some("32.0.15.6094")));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    assert_eq!(
        c.validation_text(),
        (
            false,
            "Refused: NVIDIA driver 560.94 is too old; 616.56 or newer is required".to_string()
        )
    );
}

#[test]
fn buttons_follow_admin_validation_override_and_busy() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    fake.set_admin(false);
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    assert!(c.validation_text().0); // status and checks work without admin
    assert_eq!((c.admin(), c.can_load(), c.can_remove()), (false, false, false));
    fake.set_admin(true);
    c.refresh();
    assert_eq!((c.can_load(), c.can_remove()), (true, false));
    for values in [
        vec![(VBIOS, bin(b"x"))],
        vec![("romOverride05", bin(b"x"))],
        vec![(BACKUP, sz("0x00000012"))],
    ] {
        key.set_values(with_unrelated(values.clone()));
        c.refresh();
        assert!(c.can_remove(), "{values:?}");
    }
    let _job = c.start(Action::Verify);
    assert!(c.busy());
    assert_eq!((c.can_load(), c.can_remove()), (false, false));
    c.select(5);
    assert_eq!(c.selected_index(), Some(0));
}

#[test]
fn confirmation_texts() {
    let applies = "The change applies when you restart Windows. Shutting down works too when Windows Fast Startup \
                   is off. With it on, the change waits for the next restart, and nothing has to be repeated.";
    let (fake, key) = shared(platform(
        RTX_5070,
        vec![(VBIOS, bin(b"mvolt")), (FIRMWARE, dword(0x11))],
    ));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    let path = files.file(&rom(RTX_5070, 4));
    c.choose_file(&path);
    assert_eq!(c.path(), Some(path.as_str()));
    let question = c.confirm_load().expect("a question");
    for part in [
        "GPU 0 (GPU 0, PCI 01:00.0)".to_string(),
        format!("File: {path}"),
        format!("Stored: {} bytes", chain_len()),
        "set vbios = REG_BINARY".to_string(),
        "set VixLoaderPreviousEnableGpuFirmware = REG_SZ 0x00000011".to_string(),
        "another tool".to_string(),
        en("firmware_gsp_kept").to_string(),
        applies.to_string(),
        "If the GPU does not start".to_string(),
    ] {
        assert!(question.contains(&part), "{part:?} not in {question:?}");
    }
    assert!(!question.contains("set EnableGpuFirmware"));
    let question = c.confirm_remove().expect("a question");
    assert!(question.contains("delete vbios"));
    assert!(question.contains("Note: EnableGpuFirmware was not saved by this tool"));
    assert!(question.ends_with(&format!("\n\n{applies}")));
    assert_eq!(key.calls(), vec![]);
}

#[test]
fn load_and_remove_results() {
    let restart = "Restart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. \
                   With it on, the change waits for the next restart, and nothing has to be repeated.";
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    let (ok, lines) = run(&mut c, Action::Load);
    assert!(ok);
    assert_eq!(lines[1..], [restart]);
    assert!(lines[0].starts_with(&format!("GPU 0: VBIOS loaded, {} bytes", chain_len())));
    assert_eq!(
        key.value(VBIOS),
        Some(bin(&chain(RTX_5070, 4, ImageOptions::default())))
    );
    c.refresh();
    assert!(c.can_remove());
    assert!(c.rows()[0].override_text.starts_with("loaded: "));
    let (ok, lines) = run(&mut c, Action::Unload);
    assert_eq!(
        (ok, lines),
        (true, vec!["GPU 0: override removed.".to_string(), restart.to_string()])
    );
    assert_eq!(sorted(key.values()), sorted(unrelated()));
}

#[test]
fn a_non_dword_enable_gpu_firmware_is_saved_and_put_back_in_both_languages() {
    let qword = 0x11u64.to_le_bytes();
    let files = TempDir::new();
    for (language, removed) in [
        (Language::En, "GPU 0: override removed."),
        (Language::Es, "GPU 0: reemplazo quitado."),
    ] {
        let t = |key: &str| text(language, key);
        let (fake, key) = shared(platform(RTX_5070, vec![(FIRMWARE, raw(REG_QWORD, &qword))]));
        let mut c = controller(&fake, language);
        c.choose_file(&files.file(&rom(RTX_5070, 4)));
        assert!(c.validation_text().0);
        let question = c.confirm_load().expect("a question");
        assert!(question.contains("set VixLoaderPreviousEnableGpuFirmware = REG_SZ hex(b):11,00,00,00,00,00,00,00"));
        assert!(!question.contains("set EnableGpuFirmware"));
        assert!(!question.contains(t("other_tool")));
        let (ok, lines) = run(&mut c, Action::Load);
        assert!(ok);
        assert_eq!(lines[1..], [t("restart_windows")]);
        assert_eq!(key.raw(FIRMWARE), Some((REG_QWORD, qword.to_vec()))); // left as it is
        c.refresh();
        let loaded = t("override_loaded").split(':').next().unwrap();
        assert!(c.rows()[0].override_text.starts_with(loaded));
        let question = c.confirm_remove().expect("a question");
        assert!(question.contains("set EnableGpuFirmware = REG_QWORD 0x11"));
        let (ok, lines) = run(&mut c, Action::Unload);
        assert_eq!((ok, lines[0].as_str()), (true, removed));
        assert_eq!(key.raw(FIRMWARE), Some((REG_QWORD, qword.to_vec())));
        let mut expected: BTreeSet<String> = unrelated().into_iter().map(|(name, _)| name).collect();
        expected.insert(FIRMWARE.to_string());
        assert_eq!(names(&key), expected);
    }

    // With another tool's override as well, only that note is shown: EnableGpuFirmware is not changed.
    let (fake, _) = shared(platform(
        RTX_5070,
        vec![(VBIOS, bin(b"mvolt")), (FIRMWARE, raw(REG_SZ, b"1\0"))],
    ));
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    let (_, lines) = run(&mut c, Action::Load);
    assert_eq!(lines[1..], [en("other_tool"), en("restart_windows")]);
    // A value too large to save is refused before anything is written.
    let (fake, key) = shared(platform(RTX_5070, vec![(FIRMWARE, bin(&[0; 5000]))]));
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    assert_eq!(
        c.validation_text(),
        (
            false,
            "Refused: EnableGpuFirmware holds a REG_BINARY value of 5000 bytes, too large to save (at most \
             4096); not changing it"
                .to_string()
        )
    );
    assert_eq!(c.confirm_load(), None);
    assert_eq!(key.calls(), vec![]);
}

#[test]
fn gpus_left_in_gsp_firmware_mode_by_0_3_x_are_named_and_a_load_undoes_it() {
    let old = || {
        vec![
            (VBIOS, bin(&chain(RTX_5070, 4, ImageOptions::default()))),
            (FIRMWARE, dword(0x11)),
            (BACKUP, sz("0x00000012")),
        ]
    };
    let gpus: Vec<Adapter> = (0..3)
        .map(|i| adapter(i, RTX_5070, Some(&format!("0{}:00.0", i + 1)), Some("32.0.16.1656")))
        .collect();
    let mut absent = old();
    absent[2] = (BACKUP, sz(ABSENT));
    let keys = vec![
        (gpus[0].instance_id.clone(), FakeKey::new(old())),
        (gpus[1].instance_id.clone(), FakeKey::new(unrelated())),
        (gpus[2].instance_id.clone(), FakeKey::new(absent)),
    ];
    let fake = new_platform(gpus, keys);
    let expected: Vec<String> = [0, 2]
        .iter()
        .map(|index| en("gsp_old_loader").replace("{index}", &index.to_string()))
        .collect();
    assert_eq!(Controller::new(fake, Language::En, Theme::Auto).refresh(), expected);
    let texts = [
        (
            Language::En,
            "GPU 0: GSP firmware mode is on (set by loader 0.3.2 or older). Load the VBIOS file again with this \
             version (or remove the override) and restart Windows.",
            "EnableGpuFirmware goes back to its value before the first load: 0x00000012. Loader 0.3.2 or older set \
             it to 0x11, which turns on the driver's GSP firmware mode; this version no longer turns that mode on.",
        ),
        (
            Language::Es,
            "GPU 0: el modo de firmware GSP está activado (lo activó el loader 0.3.2 o anterior). Vuelve a cargar \
             el archivo VBIOS con esta versión (o quita el reemplazo) y reinicia Windows.",
            "EnableGpuFirmware vuelve a su valor de antes de la primera carga: 0x00000012. El loader 0.3.2 o \
             anterior lo puso en 0x11, lo que activa el modo de firmware GSP del driver; esta versión ya no activa \
             ese modo.",
        ),
    ];
    let files = TempDir::new();
    for (language, line, restored) in texts {
        let (fake, key) = shared(platform(RTX_5070, old()));
        let mut c = Controller::new(fake, language, Theme::Auto);
        assert_eq!(c.refresh(), [line]);
        c.choose_file(&files.file(&rom(RTX_5070, 4)));
        let question = c.confirm_load().expect("a question");
        assert!(question.contains("  set EnableGpuFirmware = REG_DWORD 0x12"));
        assert!(question.split("\n\n").any(|part| part == restored));
        let (ok, lines) = run(&mut c, Action::Load);
        assert_eq!(
            (ok, lines[1..].to_vec()),
            (
                true,
                vec![restored.to_string(), text(language, "restart_windows").to_string()]
            )
        );
        assert_eq!(key.value(FIRMWARE), Some(dword(0x12)));
        assert_eq!(c.refresh(), Vec::<String>::new());
    }
    // 0x11 set before the first load (by mVolt+, for example) is not 0.3.x's: a note when loading, no line.
    for language in Language::ALL {
        let t = |key: &str| text(language, key);
        let (fake, key) = shared(platform(RTX_5070, vec![(FIRMWARE, dword(0x11))]));
        let mut c = controller(&fake, language);
        c.choose_file(&files.file(&rom(RTX_5070, 4)));
        let question = c.confirm_load().expect("a question");
        assert!(question.split("\n\n").any(|part| part == t("firmware_gsp_kept")));
        let (_, lines) = run(&mut c, Action::Load);
        assert_eq!(lines[1..], [t("firmware_gsp_kept"), t("restart_windows")]);
        assert_eq!(c.refresh(), Vec::<String>::new());
        assert_eq!(key.value(FIRMWARE), Some(dword(0x11)));
    }
}

#[test]
fn worker_errors_are_reported_not_raised() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    fake.set_admin(false);
    assert_eq!(
        run(&mut c, Action::Unload),
        (
            false,
            vec!["Error: administrator rights are required; run this from an elevated PowerShell".to_string()]
        )
    );
    // Python replaced open_key with one raising RuntimeError('boom'): "Unexpected error: RuntimeError: boom".
    fake.set_admin(true);
    fake.panic_on_open(Some("boom"));
    assert_eq!(
        run(&mut c, Action::Unload),
        (false, vec!["Unexpected error: panic: boom".to_string()])
    );
    assert!(!c.busy());
    assert_eq!(key.calls(), vec![]);
}

#[test]
fn verify_needs_only_a_selected_gpu_and_reports_each_state() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    fake.set_admin(false);
    fake.set_image(b"card image".to_vec());
    let mut c = controller(&fake, Language::En);
    assert!(c.can_verify()); // no file and no administrator rights needed
    let (ok, lines) = run(&mut c, Action::Verify);
    assert_eq!(
        (ok, lines[0].as_str()),
        (
            true,
            "GPU 0: no override stored: the driver should be using the card's own VBIOS. If you just removed one, \
             restart Windows first."
        )
    );
    assert!(lines[1].starts_with("Running VBIOS: 10 bytes, sha256 "));
    key.set_values([(VBIOS, bin(b"card image")), ("romOverride00", bin(b"r"))]);
    let (ok, lines) = run(&mut c, Action::Verify);
    assert_eq!(
        (ok, lines[0].as_str()),
        (
            true,
            "GPU 0: the driver is using the loaded override (verified byte by byte)."
        )
    );
    assert_eq!(lines.len(), 3); // no romOverride note: it is only for no-override
    assert!(lines[2].starts_with("Stored override: 10 bytes, sha256 "));
    key.insert(VBIOS, bin(b"card imagf"));
    let (ok, lines) = run(&mut c, Action::Verify);
    assert!(!ok);
    assert_eq!(
        lines[0],
        "GPU 0: the driver is NOT using the stored override. If you just loaded it, restart Windows; otherwise the \
         driver may have rejected it and is running the card's own VBIOS."
    );
    assert_eq!(lines[1], "First difference at offset 0x9.");
    assert_eq!(key.calls(), vec![]);
    let _job = c.start(Action::Verify);
    assert!(!c.can_verify());
    let empty = new_platform(Vec::new(), Vec::new());
    assert!(!controller(&empty, Language::En).can_verify());
}

#[test]
fn verify_texts_in_spanish_prefix_unstable_and_errors() {
    let big: Vec<u8> = (0..4001).flat_map(|_| 0..=255u8).collect();
    let (fake, key) = shared(platform(RTX_5070, vec![(VBIOS, bin(&big))]));
    fake.set_image(big[..1_024_000].to_vec());
    let mut c = controller(&fake, Language::Es);
    let (ok, lines) = run(&mut c, Action::Verify);
    assert_eq!(
        lines[..2],
        [
            "GPU 0: los primeros 1024000 bytes de la VBIOS en uso coinciden con el reemplazo cargado; el resto no \
             se puede leer."
                .to_string(),
            format!(
                "Solo se compararon los primeros 1024000 bytes: NvAPI entrega como máximo esa cantidad, y el \
                 reemplazo tiene {}.",
                big.len()
            )
        ]
    );
    assert!(ok);
    let mut english = controller(&fake, Language::En);
    assert_eq!(
        run(&mut english, Action::Verify).1[0],
        "GPU 0: the first 1024000 bytes the driver is running match the loaded override; the rest cannot be read \
         back."
    );
    let mut image = big[..1_024_000].to_vec();
    image[0] = 0xFF;
    fake.set_image(image);
    let (ok, lines) = run(&mut c, Action::Verify);
    assert_eq!(
        (ok, lines[0].as_str(), lines[2].as_str()),
        (
            false,
            "GPU 0: el driver NO está usando el reemplazo guardado. Si lo acabas de cargar, reinicia Windows; si \
             no, puede que el driver lo haya rechazado y esté usando la VBIOS de la tarjeta.",
            "Primera diferencia en el offset 0x0."
        )
    );
    key.set_values([("romOverride00", bin(b"r"))]);
    assert_eq!(
        run(&mut c, Action::Verify).1[..2],
        [
            "GPU 0: sin reemplazo guardado: el driver debería usar la VBIOS de la tarjeta. Si acabas de quitar uno, \
             reinicia Windows primero.",
            "También hay registros romOverride (mVolt+) guardados; el driver puede aplicarlos."
        ]
    );
    fake.queue_image(Ok(b"a".to_vec()));
    fake.queue_image(Ok(b"b".to_vec()));
    let (ok, lines) = run(&mut c, Action::Verify);
    assert_eq!(
        (ok, lines[0].as_str()),
        (
            false,
            "GPU 0: dos lecturas de la VBIOS en uso dieron resultados distintos; inténtalo de nuevo."
        )
    );
    fake.queue_image(Err(Error::new("this NVIDIA driver has no NvAPI_GPU_GetVbiosImage")));
    assert_eq!(
        run(&mut c, Action::Verify),
        (
            false,
            vec!["Error: this NVIDIA driver has no NvAPI_GPU_GetVbiosImage".to_string()]
        )
    );
}

#[test]
fn window_verify_button_state() {
    // test_window_verify_button without Tk: the texts and states the window reads from the controller.
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    fake.set_admin(false);
    let mut c = Controller::new(fake, Language::Es, Theme::Dark);
    assert_eq!(c.text("verify"), "Verificar");
    assert!(!c.can_verify()); // no GPU listed yet
    c.refresh();
    assert!(c.can_verify());
    assert!(!c.can_load());
    let job = c.start(Action::Verify);
    assert_eq!(job.action(), Action::Verify);
    assert!(!c.can_verify());
    assert_eq!(c.text("verifying"), "Leyendo la VBIOS que está usando el driver...");
    assert_eq!(c.text("working"), "Trabajando...");
    assert_eq!(c.theme(), Theme::Dark);
}

fn same_model() -> (Vec<Adapter>, Vec<(String, FakeKey)>, Arc<FakePlatform>) {
    let gpus: Vec<Adapter> = (0..3)
        .map(|i| adapter(i, RTX_5070, Some(&format!("0{}:00.0", i + 1)), Some("32.0.16.1656")))
        .collect();
    let keys: Vec<(String, FakeKey)> = gpus
        .iter()
        .map(|gpu| (gpu.instance_id.clone(), FakeKey::new(unrelated())))
        .collect();
    let fake = new_platform(gpus.clone(), keys.clone());
    (gpus, keys, fake)
}

fn with_index(gpu: &Adapter, index: usize) -> Adapter {
    Adapter { index, ..gpu.clone() }
}

#[test]
fn the_selected_device_is_written_even_when_the_indexes_shift() {
    let (gpus, keys, fake) = same_model();
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.select(1);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    assert!(c.confirm_load().expect("a question").contains("PCI 02:00.0"));
    // 01:00.0 fell off the bus.
    fake.set_adapters(vec![with_index(&gpus[1], 0), with_index(&gpus[2], 1)]);
    let (ok, lines) = run(&mut c, Action::Load);
    assert!(ok);
    assert!(lines[0].starts_with("GPU 0: VBIOS loaded"));
    let written: Vec<bool> = keys.iter().map(|(_, key)| key.value(VBIOS).is_some()).collect();
    assert_eq!(written, [false, true, false]);
}

#[test]
fn a_gpu_that_goes_away_stays_selected_and_is_not_replaced() {
    let (gpus, _, fake) = same_model();
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.select(1);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    fake.set_adapters(vec![gpus[0].clone(), with_index(&gpus[2], 1)]);
    c.refresh();
    assert_eq!(c.selected_index(), None);
    assert_eq!(
        (c.validation_text(), c.can_load(), c.confirm_load()),
        ((false, "Select a GPU.".to_string()), false, None)
    );
    fake.set_adapters(gpus);
    c.refresh();
    assert_eq!(
        (selected_location(&c).as_deref(), c.can_load()),
        (Some("02:00.0"), true)
    );
}

#[test]
fn load_confirmation_comes_from_a_fresh_dry_run() {
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    key.insert(VBIOS, bin(b"mvolt"));
    key.insert("romOverride00", bin(b"r"));
    let question = c.confirm_load().expect("a question");
    assert!(question.contains("another tool"));
    assert!(question.contains("delete romOverride00"));
    key.insert(BACKUP, dword(0x12));
    assert_eq!(c.confirm_load(), None);
    assert!(!c.can_load());
}

#[test]
fn malformed_backup_can_be_removed_and_hints_name_the_window_buttons() {
    let (fake, _) = shared(platform(RTX_5070, vec![(BACKUP, dword(0x12))]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::Es);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    assert!(c
        .validation_text()
        .1
        .ends_with("run unload first En esta ventana: usa primero Quitar reemplazo."));
    assert!(c.can_remove());
    assert_eq!(c.rows()[0].override_text, "solo el valor guardado de EnableGpuFirmware");
    assert!(c
        .confirm_remove()
        .expect("a question")
        .contains("delete VixLoaderPreviousEnableGpuFirmware"));

    // FailingKey: set() raises OSError(5, 'Access is denied').
    let (fake, key) = shared(platform(RTX_5070, vec![]));
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    key.on_set(|_, _| Some(Error::new("[Errno 5] Access is denied")));
    let (ok, lines) = run(&mut c, Action::Load);
    assert!(!ok);
    assert!(
        lines[0].ends_with(
            "before restarting Windows In this window: click Refresh, then Remove override, before restarting \
             Windows."
        ),
        "{lines:?}"
    );
}

#[test]
fn run_never_panics() {
    // Python replaced operations.load with one returning {} ("Unexpected error: KeyError: 'gpu'"); here the
    // platform panics inside load.
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::En);
    c.choose_file(&files.file(&rom(RTX_5070, 4)));
    fake.panic_on_open(Some("boom"));
    assert_eq!(
        run(&mut c, Action::Load),
        (false, vec!["Unexpected error: panic: boom".to_string()])
    );
    // A job runs in a worker thread.
    fake.panic_on_open(None);
    let job = c.start(Action::Verify);
    let result = std::thread::spawn(move || job.run()).join().expect("the worker");
    c.finish();
    assert!(result.0);
    // Load without a file read (the window does not offer it).
    let mut empty = controller(&fake, Language::En);
    assert_eq!(
        run(&mut empty, Action::Load),
        (false, vec!["Unexpected error: no VBIOS file was read".to_string()])
    );
}

#[test]
fn status_failure_is_a_message() {
    let fake = new_platform(Vec::new(), Vec::new());
    fake.fail_adapters(Some(Error::new("[Errno 13] SetupAPI failed")));
    let mut c = Controller::new(fake, Language::En, Theme::Auto);
    assert_eq!(c.refresh(), ["Cannot list the GPUs: [Errno 13] SetupAPI failed"]);
    assert_eq!(c.rows(), Vec::<Row>::new());
}

/// The {name} fields of a text (string.Formatter().parse).
fn fields(text: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let close = after.find('}').expect("a closing brace");
        found.insert(after[..close].to_string());
        rest = &after[close + 1..];
    }
    found
}

#[test]
fn spanish_has_every_key_with_the_same_placeholders() {
    let keys = |language| -> Vec<&str> { texts(language).iter().map(|(key, _)| *key).collect() };
    assert_eq!(keys(Language::En), keys(Language::Es));
    assert_eq!(keys(Language::En).len(), 70);
    for (key, english) in texts(Language::En) {
        assert_eq!(fields(text(Language::Es, key)), fields(english), "{key}");
    }
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    fake.set_admin(false);
    let c = controller(&fake, Language::Es);
    assert_eq!(c.rows()[0].override_text, "ninguno");
    assert_eq!(c.validation_text(), (false, "Elige un archivo VBIOS.".to_string()));
    assert_eq!(
        (language_for(0x0C0A), language_for(0x080A), language_for(0x0409)),
        (Language::Es, Language::Es, Language::En)
    );
}

#[test]
#[should_panic(expected = "no text \"bogus\" in en")]
fn an_unknown_text_key_is_a_programming_error() {
    let _ = text(Language::En, "bogus");
}

#[test]
fn language_switch_rewrites_texts_and_the_refusal() {
    let (fake, _) = shared(platform(RTX_5070, vec![]));
    let files = TempDir::new();
    let mut c = controller(&fake, Language::Es);
    c.choose_file(&files.file(&[0; 0x400]));
    let (ok, shown) = c.validation_text();
    assert!(!ok);
    assert!(shown.starts_with("Rechazado: "));
    c.set_language(Language::En);
    assert_eq!(
        (c.language(), c.rows()[0].override_text.as_str()),
        (Language::En, "none")
    );
    assert!(c.validation_text().1.starts_with("Refused: "));
    c.choose_file(&files.missing());
    c.set_language(Language::Es);
    assert!(c.validation_text().1.starts_with("Rechazado: No se puede leer "));
    let _job = c.start(Action::Load); // not while a load or unload runs
    c.set_language(Language::En);
    assert_eq!(c.language(), Language::Es);
}

#[test]
fn theme_auto_follows_the_windows_app_mode() {
    let empty = new_platform(Vec::new(), Vec::new());
    let mut c = Controller::new(empty, Language::En, Theme::Auto);
    assert_eq!(c.theme(), Theme::Auto);
    c.set_theme(Theme::Light);
    assert_eq!(c.theme(), Theme::Light);
    assert_eq!(
        (
            resolve_theme(Theme::Auto, || true),
            resolve_theme(Theme::Auto, || false)
        ),
        (Theme::Dark, Theme::Light)
    );
    for mode in [Theme::Light, Theme::Dark] {
        assert_eq!(resolve_theme(mode, || mode == Theme::Light), mode);
    }
    for theme in Theme::ALL {
        assert_eq!(Theme::from_code(theme.code()), Some(theme));
        assert!(!text(Language::Es, theme.text_key()).is_empty());
    }
    assert_eq!(Theme::from_code("blue"), None);
    for language in Language::ALL {
        assert_eq!(Language::from_code(language.code()), Some(language));
    }
    assert_eq!(Language::Es.native_name(), "Español");
}

#[test]
fn format_fills_named_fields() {
    assert_eq!(format("a {x} b {y}{x}", &[("x", "1"), ("y", "2")]), "a 1 b 21");
    assert_eq!(format("no fields", &[]), "no fields");
    assert_eq!(
        format(
            en("valid"),
            &[("index", "0"), ("family", "f"), ("size", "3"), ("sha256", "ab")]
        ),
        "OK for GPU 0: f, 3 bytes to store, sha256 ab."
    );
}
