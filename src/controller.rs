//! State of the loader window: selection, validation, button enablement and texts (controller.py). No Win32 here.
//!
//! Everything goes through `operations`: the list comes from status(), a file is checked with load(dry_run), and
//! Load / Remove override / Verify run load(), unload() and verify(). `gui` only draws, and runs a `Job` in a
//! worker thread while the controller is busy.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use crate::cli::{os_error_text, panic_text, step_text};
use crate::error::Result;
use crate::json::{Map, Value};
use crate::operations::{load, status, unload, verify, Selection, MIN_DRIVER, VERIFY_OK};
use crate::platform::Platform;

/// The window languages; each is named in itself in the Language menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    En,
    Es,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::En, Language::Es];

    /// "en" or "es".
    pub fn code(self) -> &'static str {
        match self {
            Language::En => "en",
            Language::Es => "es",
        }
    }

    pub fn from_code(code: &str) -> Option<Language> {
        Language::ALL.into_iter().find(|l| l.code() == code)
    }

    /// "English" or "Español".
    pub fn native_name(self) -> &'static str {
        match self {
            Language::En => "English",
            Language::Es => "Español",
        }
    }
}

/// Spanish when the primary language of a Windows LANGID (GetUserDefaultUILanguage) is Spanish (0x0a), else English.
pub fn language_for(langid: u16) -> Language {
    if langid & 0x3FF == 0x0A {
        Language::Es
    } else {
        Language::En
    }
}

/// The theme setting: auto follows the Windows app mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Auto, Theme::Light, Theme::Dark];

    /// "auto", "light" or "dark".
    pub fn code(self) -> &'static str {
        match self {
            Theme::Auto => "auto",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    pub fn from_code(code: &str) -> Option<Theme> {
        Theme::ALL.into_iter().find(|t| t.code() == code)
    }

    /// The text key of the Theme menu entry: "theme_auto", "theme_light" or "theme_dark".
    pub fn text_key(self) -> &'static str {
        match self {
            Theme::Auto => "theme_auto",
            Theme::Light => "theme_light",
            Theme::Dark => "theme_dark",
        }
    }
}

/// Light or Dark: Auto follows `dark_mode()` (true when Windows asks apps for the dark mode).
pub fn resolve_theme(mode: Theme, dark_mode: impl FnOnce() -> bool) -> Theme {
    match mode {
        Theme::Auto => {
            if dark_mode() {
                Theme::Dark
            } else {
                Theme::Light
            }
        }
        other => other,
    }
}

// controller.STRINGS of loader 0.4.x, copied verbatim in their order.
const EN: [(&str, &str); 70] = [
    ("title", "Windows VBIOS Vix Loader"),
    (
        "not_admin",
        "Not running as administrator: you can see the status, but Load and Remove override are disabled. Close this \
         window and start the program as administrator.",
    ),
    ("gpus", "NVIDIA GPUs"),
    ("col_index", "#"),
    ("col_name", "Name"),
    ("col_pci", "PCI ID"),
    ("col_location", "Location"),
    ("col_driver", "Driver"),
    ("col_override", "Override"),
    ("refresh", "Refresh"),
    ("file", "VBIOS file"),
    ("file_button", "VBIOS file..."),
    ("file_types", "VBIOS files"),
    ("all_files", "All files"),
    ("load", "Load"),
    ("remove", "Remove override"),
    ("verify", "Verify"),
    ("messages", "Messages"),
    ("working", "Working..."),
    ("verifying", "Reading the VBIOS the driver is running..."),
    ("override_none", "none"),
    ("override_loaded", "loaded: {size} bytes, sha256 {sha}..."),
    ("override_other", "other tool's override: {size} bytes"),
    ("override_bad", "vbios value of unexpected type {type}"),
    ("override_records", "romOverride records (mVolt+): {count}"),
    ("override_backup", "saved EnableGpuFirmware value only"),
    ("override_error", "cannot read the driver key"),
    ("driver_unknown", "unknown"),
    ("driver_too_old", "{version} (too old, {minimum} or newer needed)"),
    ("no_gpu", "No NVIDIA GPU found."),
    ("choose_gpu", "Select a GPU."),
    ("choose_file", "Choose a VBIOS file."),
    ("cannot_read", "Cannot read {path}: {error}"),
    (
        "valid",
        "OK for GPU {index}: {family}, {size} bytes to store, sha256 {sha256}.",
    ),
    ("refused", "Refused: {reason}"),
    ("confirm_load_title", "Load VBIOS override"),
    (
        "confirm_load",
        "Load this VBIOS on GPU {index} ({name}, PCI {location})?\n\nFile: {path}\nStored: {size} bytes of the image \
         chain ({family}), sha256 {sha256}\n\nRegistry changes in the GPU's driver key:\n{steps}",
    ),
    ("confirm_remove_title", "Remove VBIOS override"),
    (
        "confirm_remove",
        "Remove the override from GPU {index} ({name}, PCI {location})?\n\nRegistry changes in the GPU's driver \
         key:\n{steps}",
    ),
    (
        "other_tool",
        "This GPU has an override written by another tool (for example mVolt+). It is replaced and will not be \
         restored.",
    ),
    (
        "firmware_restored",
        "EnableGpuFirmware goes back to its value before the first load: {saved}. Loader 0.3.2 or older set it to \
         0x11, which turns on the driver's GSP firmware mode; this version no longer turns that mode on.",
    ),
    (
        "firmware_gsp_kept",
        "GSP firmware mode stays on because EnableGpuFirmware was already 0x11 before the first load (for example set \
         by mVolt+). For the driver's default mode, remove the override (Remove override puts 0x11 back), delete \
         EnableGpuFirmware, load the file again and restart Windows.",
    ),
    (
        "gsp_old_loader",
        "GPU {index}: GSP firmware mode is on (set by loader 0.3.2 or older). Load the VBIOS file again with this \
         version (or remove the override) and restart Windows.",
    ),
    (
        "applies_on_restart",
        "The change applies when you restart Windows. Shutting down works too when Windows Fast Startup is off. With \
         it on, the change waits for the next restart, and nothing has to be repeated.",
    ),
    (
        "risk",
        "A modified VBIOS can damage hardware. If the GPU does not start, see \"If the GPU does not start\" in the \
         README: boot into Safe Mode and remove the override.",
    ),
    ("loaded", "GPU {index}: VBIOS loaded, {size} bytes, sha256 {sha256}."),
    ("removed", "GPU {index}: override removed."),
    (
        "restart_windows",
        "Restart Windows to apply the change. Shutting down works too when Windows Fast Startup is off. With it on, \
         the change waits for the next restart, and nothing has to be repeated.",
    ),
    (
        "verify_override-active",
        "GPU {index}: the driver is using the loaded override (verified byte by byte).",
    ),
    (
        "verify_override-active-prefix",
        "GPU {index}: the first {compared} bytes the driver is running match the loaded override; the rest cannot be \
         read back.",
    ),
    (
        "verify_override-not-active",
        "GPU {index}: the driver is NOT using the stored override. If you just loaded it, restart Windows; otherwise \
         the driver may have rejected it and is running the card's own VBIOS.",
    ),
    (
        "verify_no-override",
        "GPU {index}: no override stored: the driver should be using the card's own VBIOS. If you just removed one, \
         restart Windows first.",
    ),
    (
        "verify_unstable",
        "GPU {index}: two reads of the running VBIOS differ; try again.",
    ),
    (
        "verify_prefix",
        "Only the first {compared} bytes were compared: NvAPI returns at most that many, and the override has {size}.",
    ),
    ("verify_difference", "First difference at offset {offset}."),
    (
        "verify_records",
        "romOverride records (mVolt+) are stored too; the driver may apply them to the card's VBIOS.",
    ),
    ("verify_running", "Running VBIOS: {size} bytes, sha256 {sha256}."),
    ("verify_stored", "Stored override: {size} bytes, sha256 {sha256}."),
    ("note", "Note: {message}"),
    ("error", "Error: {message}"),
    ("unexpected", "Unexpected error: {message}"),
    ("status_failed", "Cannot list the GPUs: {message}"),
    (
        "hint_remove",
        "In this window: click Refresh, then Remove override, before restarting Windows.",
    ),
    ("hint_remove_first", "In this window: use Remove override first."),
    (
        "hint_refresh",
        "In this window: click Refresh and select the GPU again.",
    ),
    ("language", "Language"),
    ("theme", "Theme"),
    ("theme_auto", "System"),
    ("theme_light", "Light"),
    ("theme_dark", "Dark"),
];

const ES: [(&str, &str); 70] = [
    ("title", "Windows VBIOS Vix Loader"),
    (
        "not_admin",
        "No se está ejecutando como administrador: puedes ver el estado, pero Cargar y Quitar reemplazo están \
         desactivados. Cierra esta ventana y abre el programa como administrador.",
    ),
    ("gpus", "GPUs NVIDIA"),
    ("col_index", "#"),
    ("col_name", "Nombre"),
    ("col_pci", "ID PCI"),
    ("col_location", "Ubicación"),
    ("col_driver", "Driver"),
    ("col_override", "Reemplazo"),
    ("refresh", "Actualizar"),
    ("file", "Archivo VBIOS"),
    ("file_button", "Archivo VBIOS..."),
    ("file_types", "Archivos VBIOS"),
    ("all_files", "Todos los archivos"),
    ("load", "Cargar"),
    ("remove", "Quitar reemplazo"),
    ("verify", "Verificar"),
    ("messages", "Mensajes"),
    ("working", "Trabajando..."),
    ("verifying", "Leyendo la VBIOS que está usando el driver..."),
    ("override_none", "ninguno"),
    ("override_loaded", "cargado: {size} bytes, sha256 {sha}..."),
    ("override_other", "reemplazo de otra herramienta: {size} bytes"),
    ("override_bad", "valor vbios de tipo inesperado {type}"),
    ("override_records", "registros romOverride (mVolt+): {count}"),
    ("override_backup", "solo el valor guardado de EnableGpuFirmware"),
    ("override_error", "no se puede leer la clave del driver"),
    ("driver_unknown", "desconocido"),
    (
        "driver_too_old",
        "{version} (muy antiguo, se necesita {minimum} o más nuevo)",
    ),
    ("no_gpu", "No se encontró ninguna GPU NVIDIA."),
    ("choose_gpu", "Elige una GPU."),
    ("choose_file", "Elige un archivo VBIOS."),
    ("cannot_read", "No se puede leer {path}: {error}"),
    (
        "valid",
        "Válido para la GPU {index}: {family}, {size} bytes a guardar, sha256 {sha256}.",
    ),
    ("refused", "Rechazado: {reason}"),
    ("confirm_load_title", "Cargar VBIOS de reemplazo"),
    (
        "confirm_load",
        "¿Cargar esta VBIOS en la GPU {index} ({name}, PCI {location})?\n\nArchivo: {path}\nSe guardan: {size} bytes \
         de la cadena de imágenes ({family}), sha256 {sha256}\n\nCambios en la clave de registro del driver de la \
         GPU:\n{steps}",
    ),
    ("confirm_remove_title", "Quitar VBIOS de reemplazo"),
    (
        "confirm_remove",
        "¿Quitar el reemplazo de la GPU {index} ({name}, PCI {location})?\n\nCambios en la clave de registro del \
         driver de la GPU:\n{steps}",
    ),
    (
        "other_tool",
        "Esta GPU tiene un reemplazo escrito por otra herramienta (por ejemplo mVolt+). Se reemplaza y no se va a \
         restaurar.",
    ),
    (
        "firmware_restored",
        "EnableGpuFirmware vuelve a su valor de antes de la primera carga: {saved}. El loader 0.3.2 o anterior lo \
         puso en 0x11, lo que activa el modo de firmware GSP del driver; esta versión ya no activa ese modo.",
    ),
    (
        "firmware_gsp_kept",
        "El modo de firmware GSP sigue activado porque EnableGpuFirmware ya estaba en 0x11 antes de la primera carga \
         (por ejemplo, puesto por mVolt+). Para usar el modo predeterminado del driver, quita el reemplazo (Quitar \
         reemplazo lo vuelve a poner en 0x11), borra EnableGpuFirmware, vuelve a cargar el archivo y reinicia Windows.",
    ),
    (
        "gsp_old_loader",
        "GPU {index}: el modo de firmware GSP está activado (lo activó el loader 0.3.2 o anterior). Vuelve a cargar \
         el archivo VBIOS con esta versión (o quita el reemplazo) y reinicia Windows.",
    ),
    (
        "applies_on_restart",
        "El cambio se aplica cuando reinicies Windows. Apagar también sirve si el inicio rápido de Windows está \
         desactivado. Si está activado, el cambio espera al siguiente reinicio, y no hay que repetir nada.",
    ),
    (
        "risk",
        "Una VBIOS modificada puede dañar el hardware. Si la GPU no arranca, revisa \"Si la GPU no arranca\" en el \
         README: arranca en Modo seguro y quita el reemplazo.",
    ),
    ("loaded", "GPU {index}: VBIOS cargada, {size} bytes, sha256 {sha256}."),
    ("removed", "GPU {index}: reemplazo quitado."),
    (
        "restart_windows",
        "Reinicia Windows para aplicar el cambio. Apagar también sirve si el inicio rápido de Windows está \
         desactivado. Si está activado, el cambio espera al siguiente reinicio, y no hay que repetir nada.",
    ),
    (
        "verify_override-active",
        "GPU {index}: el driver está usando el reemplazo cargado (verificado byte a byte).",
    ),
    (
        "verify_override-active-prefix",
        "GPU {index}: los primeros {compared} bytes de la VBIOS en uso coinciden con el reemplazo cargado; el resto \
         no se puede leer.",
    ),
    (
        "verify_override-not-active",
        "GPU {index}: el driver NO está usando el reemplazo guardado. Si lo acabas de cargar, reinicia Windows; si \
         no, puede que el driver lo haya rechazado y esté usando la VBIOS de la tarjeta.",
    ),
    (
        "verify_no-override",
        "GPU {index}: sin reemplazo guardado: el driver debería usar la VBIOS de la tarjeta. Si acabas de quitar uno, \
         reinicia Windows primero.",
    ),
    (
        "verify_unstable",
        "GPU {index}: dos lecturas de la VBIOS en uso dieron resultados distintos; inténtalo de nuevo.",
    ),
    (
        "verify_prefix",
        "Solo se compararon los primeros {compared} bytes: NvAPI entrega como máximo esa cantidad, y el reemplazo \
         tiene {size}.",
    ),
    ("verify_difference", "Primera diferencia en el offset {offset}."),
    (
        "verify_records",
        "También hay registros romOverride (mVolt+) guardados; el driver puede aplicarlos.",
    ),
    ("verify_running", "VBIOS en uso: {size} bytes, sha256 {sha256}."),
    ("verify_stored", "Reemplazo guardado: {size} bytes, sha256 {sha256}."),
    ("note", "Nota: {message}"),
    ("error", "Error: {message}"),
    ("unexpected", "Error inesperado: {message}"),
    ("status_failed", "No se pueden listar las GPUs: {message}"),
    (
        "hint_remove",
        "En esta ventana: pulsa Actualizar y luego Quitar reemplazo antes de reiniciar Windows.",
    ),
    ("hint_remove_first", "En esta ventana: usa primero Quitar reemplazo."),
    (
        "hint_refresh",
        "En esta ventana: pulsa Actualizar y vuelve a elegir la GPU.",
    ),
    ("language", "Idioma"),
    ("theme", "Tema"),
    ("theme_auto", "Sistema"),
    ("theme_light", "Claro"),
    ("theme_dark", "Oscuro"),
];

/// Every text of a language as (key, text), in the order of controller.STRINGS.
pub fn texts(language: Language) -> &'static [(&'static str, &'static str)] {
    match language {
        Language::En => &EN,
        Language::Es => &ES,
    }
}

/// The text of `key` in `language` (controller.STRINGS). Keys used by the window: title, not_admin, gpus,
/// col_index, col_name, col_pci, col_location, col_driver, col_override, refresh, file, file_button, file_types,
/// all_files, load, remove, verify, messages, working, verifying, confirm_load_title, confirm_remove_title,
/// unexpected ("Unexpected error: {message}"), language, theme, theme_auto, theme_light, theme_dark.
/// Panics on an unknown key (a programming error; a test checks every key in both languages).
pub fn text(language: Language, key: &str) -> &'static str {
    match texts(language).iter().find(|(name, _)| *name == key) {
        Some((_, text)) => text,
        None => panic!("no text {key:?} in {}", language.code()),
    }
}

/// str.format(**values) for the {name} fields of the texts (they have no other braces).
fn format(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        let name = &after[..close];
        match values.iter().find(|(key, _)| *key == name) {
            Some((_, value)) => out.push_str(value),
            None => {
                debug_assert!(false, "no value for {{{name}}}");
                out.push_str(&rest[open..open + close + 2]);
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// operations names command-line steps in its errors; the window adds what to do here instead (HINTS).
const HINTS: [(&str, &str); 4] = [
    ("registry may be partly changed", "hint_remove"),
    ("read back differ", "hint_remove"),
    ("run unload first", "hint_remove_first"),
    ("run status to list them", "hint_refresh"),
];

/// Controller._reason: the error text and the hint of the first phrase it contains.
fn reason(language: Language, message: &str) -> String {
    match HINTS.iter().find(|(phrase, _)| message.contains(phrase)) {
        Some((_, key)) => format!("{message} {}", text(language, key)),
        None => message.to_string(),
    }
}

/// controller.MINIMUM: the oldest supported NVIDIA driver, "616.56".
fn minimum() -> String {
    format!("{}.{:02}", MIN_DRIVER.0, MIN_DRIVER.1)
}

static NULL: Value = Value::Null;

/// `mapping[key]` of a report (null for a key that is missing).
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

/// `mapping[key]` of a report map.
fn entry<'a>(map: &'a Map, key: &str) -> &'a Value {
    map.get(key).unwrap_or(&NULL)
}

/// f"{value}" for the values the reports hold.
fn py_str(value: &Value) -> String {
    match value {
        Value::Null => "None".to_string(),
        Value::Bool(true) => "True".to_string(),
        Value::Bool(false) => "False".to_string(),
        Value::Int(number) => number.to_string(),
        Value::Str(text) => text.clone(),
        other => other.to_json_pretty(),
    }
}

/// What a worker runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Load,
    Unload,
    Verify,
}

/// One line of the GPU list: (index, name, PCI ID, location, driver, override) per GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub index: usize,
    pub name: String,
    pub pci_id: String,
    /// The location, or "?".
    pub location: String,
    /// The NVIDIA version, "unknown"/"desconocido", or "NNN.NN (too old, 616.56 or newer needed)".
    pub driver: String,
    pub override_text: String,
}

/// The result of the dry run for the selected GPU and the chosen file (controller.validation).
enum Validation {
    /// The load --dry-run report.
    Valid(Map),
    /// Why the file cannot be loaded, with the window's hint.
    Refused(String),
}

/// The window state (controller.Controller).
pub struct Controller {
    platform: Arc<dyn Platform>,
    language: Language,
    theme: Theme,
    /// status() entries as of the last refresh.
    gpus: Vec<Value>,
    admin: bool,
    /// The device instance id of the selected GPU: it stays chosen when the indexes change or the GPU goes away.
    selected: Option<String>,
    path: Option<String>,
    data: Option<Arc<Vec<u8>>>,
    file_error: Option<String>,
    /// None until both a GPU and a file are chosen.
    validation: Option<Validation>,
    busy: bool,
}

impl Controller {
    pub fn new(platform: Arc<dyn Platform>, language: Language, theme: Theme) -> Controller {
        Controller {
            platform,
            language,
            theme,
            gpus: Vec::new(),
            admin: false,
            selected: None,
            path: None,
            data: None,
            file_error: None,
            validation: None,
            busy: false,
        }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    /// The theme setting (auto, light or dark); the window resolves auto with `resolve_theme`.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// `text(self.language(), key)`.
    pub fn text(&self, key: &str) -> &'static str {
        text(self.language, key)
    }

    /// Administrator rights, as read by the last refresh.
    pub fn admin(&self) -> bool {
        self.admin
    }

    /// True while a Job runs (between `start` and `finish`).
    pub fn busy(&self) -> bool {
        self.busy
    }

    /// The chosen VBIOS file, as passed to `choose_file`.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Read status and admin rights again; returns message lines for the Messages area.
    pub fn refresh(&mut self) -> Vec<String> {
        let mut lines = Vec::new();
        self.admin = self.platform.is_admin();
        match status(&*self.platform) {
            Ok(gpus) => self.gpus = gpus,
            Err(error) => {
                self.gpus = Vec::new();
                lines.push(format(self.text("status_failed"), &[("message", error.message())]));
            }
        }
        for gpu in &self.gpus {
            if field(field(gpu, "override"), "gsp_forced_by_old_loader").truthy() {
                lines.push(format(
                    self.text("gsp_old_loader"),
                    &[("index", &py_str(field(gpu, "index")))],
                ));
            }
        }
        // Only before any choice: a GPU that went away stays chosen.
        if self.selected.is_none() {
            if let Some(first) = self.gpus.first() {
                self.selected = field(first, "instance_id").as_str().map(str::to_string);
            }
        }
        self.validate();
        lines
    }

    /// Switch the texts; the file is checked again so its refusal reads in the new language. Ignored while busy.
    pub fn set_language(&mut self, language: Language) {
        if self.busy || language == self.language {
            return;
        }
        self.language = language;
        match self.path.clone() {
            Some(path) => self.choose_file(&path),
            None => self.validate(),
        }
    }

    /// The selected GPU's status entry when it is in the current list (controller.gpu).
    fn gpu(&self) -> Option<&Value> {
        let selected = self.selected.as_deref()?;
        self.gpus
            .iter()
            .find(|gpu| field(gpu, "instance_id").as_str() == Some(selected))
    }

    /// The status index of the selected GPU when it is in the current list (controller.gpu), else None.
    pub fn selected_index(&self) -> Option<usize> {
        self.gpu()
            .and_then(|gpu| field(gpu, "index").as_i64())
            .and_then(|index| usize::try_from(index).ok())
    }

    /// Select the GPU with this status index. Ignored while busy.
    pub fn select(&mut self, index: usize) {
        if self.busy {
            return;
        }
        let chosen = self
            .gpus
            .iter()
            .find(|gpu| field(gpu, "index").as_i64() == i64::try_from(index).ok())
            .and_then(|gpu| field(gpu, "instance_id").as_str().map(str::to_string));
        if chosen != self.selected {
            self.selected = chosen;
            self.validate();
        }
    }

    /// Read and check the file at `path` (a read error becomes the validation text).
    pub fn choose_file(&mut self, path: &str) {
        self.path = Some(path.to_string());
        self.data = None;
        self.file_error = None;
        match std::fs::read(path) {
            Ok(data) => self.data = Some(Arc::new(data)),
            Err(error) => {
                self.file_error = Some(format(
                    self.text("cannot_read"),
                    &[("path", path), ("error", &os_error_text(&error))],
                ));
            }
        }
        self.validate();
    }

    /// The same checks as `load --dry-run`, on the selected GPU and the chosen file.
    fn validate(&mut self) {
        self.validation = None;
        if let Some(error) = &self.file_error {
            self.validation = Some(Validation::Refused(error.clone()));
        } else if let (Some(_), Some(data)) = (self.gpu(), &self.data) {
            let selection = Selection {
                instance_id: self.selected.clone(),
                ..Selection::default()
            };
            self.validation = Some(match load(&*self.platform, data, &selection, true) {
                Ok(report) => Validation::Valid(report),
                Err(error) => Validation::Refused(reason(self.language, error.message())),
            });
        }
    }

    fn override_text(&self, gpu: &Value) -> String {
        if field(gpu, "error").truthy() {
            return self.text("override_error").to_string();
        }
        let override_ = field(gpu, "override");
        let vbios = field(override_, "vbios");
        let saved = field(override_, "saved_previous_enable_gpu_firmware");
        let mut parts = Vec::new();
        if !vbios.is_null() {
            let size = py_str(field(vbios, "size"));
            if field(vbios, "size").is_null() {
                parts.push(format(
                    self.text("override_bad"),
                    &[("type", &py_str(field(vbios, "type")))],
                ));
            } else if !saved.is_null() {
                let sha: String = py_str(field(vbios, "sha256")).chars().take(12).collect();
                parts.push(format(self.text("override_loaded"), &[("size", &size), ("sha", &sha)]));
            } else {
                parts.push(format(self.text("override_other"), &[("size", &size)]));
            }
        }
        let records = field(override_, "rom_override_records");
        if records.truthy() {
            let count = records.as_array().map_or(0, Vec::len).to_string();
            parts.push(format(self.text("override_records"), &[("count", &count)]));
        }
        if parts.is_empty() && !saved.is_null() {
            parts.push(self.text("override_backup").to_string());
        }
        if parts.is_empty() {
            self.text("override_none").to_string()
        } else {
            parts.join("; ")
        }
    }

    fn driver_text(&self, gpu: &Value) -> String {
        let version = field(gpu, "nvidia_driver_version");
        if version.is_null() {
            return self.text("driver_unknown").to_string();
        }
        if field(gpu, "driver_supported").truthy() {
            py_str(version)
        } else {
            format(
                self.text("driver_too_old"),
                &[("version", &py_str(version)), ("minimum", &minimum())],
            )
        }
    }

    /// The GPU list.
    pub fn rows(&self) -> Vec<Row> {
        self.gpus
            .iter()
            .map(|gpu| {
                let location = field(gpu, "location");
                Row {
                    index: field(gpu, "index")
                        .as_i64()
                        .and_then(|index| usize::try_from(index).ok())
                        .unwrap_or_default(),
                    name: py_str(field(gpu, "name")),
                    pci_id: py_str(field(gpu, "pci_id")),
                    location: if location.truthy() {
                        py_str(location)
                    } else {
                        "?".to_string()
                    },
                    driver: self.driver_text(gpu),
                    override_text: self.override_text(gpu),
                }
            })
            .collect()
    }

    /// (ok, text) for the line under the file.
    pub fn validation_text(&self) -> (bool, String) {
        if self.gpus.is_empty() {
            return (false, self.text("no_gpu").to_string());
        }
        if self.gpu().is_none() {
            return (false, self.text("choose_gpu").to_string());
        }
        match &self.validation {
            None => (false, self.text("choose_file").to_string()),
            Some(Validation::Refused(reason)) => (false, format(self.text("refused"), &[("reason", reason)])),
            Some(Validation::Valid(report)) => {
                let payload = entry(report, "payload");
                let text = format(
                    self.text("valid"),
                    &[
                        ("index", &py_str(field(entry(report, "gpu"), "index"))),
                        ("family", &py_str(field(payload, "family"))),
                        ("size", &py_str(field(payload, "size"))),
                        ("sha256", &py_str(field(payload, "sha256"))),
                    ],
                );
                (true, text)
            }
        }
    }

    pub fn can_load(&self) -> bool {
        self.admin && !self.busy && matches!(self.validation, Some(Validation::Valid(_)))
    }

    pub fn can_remove(&self) -> bool {
        let Some(gpu) = self.gpu() else {
            return false;
        };
        if !self.admin || self.busy || field(gpu, "error").truthy() {
            return false;
        }
        let override_ = field(gpu, "override");
        !field(override_, "vbios").is_null()
            || field(override_, "rom_override_records").truthy()
            || !field(override_, "saved_previous_enable_gpu_firmware").is_null()
    }

    /// Verify only reads (the registry and NvAPI), so it needs no administrator rights.
    pub fn can_verify(&self) -> bool {
        !self.busy && self.gpu().is_some()
    }

    /// The question asked before Load, from a fresh dry run; None when Load is not possible now.
    pub fn confirm_load(&mut self) -> Option<String> {
        self.validate();
        if !self.can_load() {
            return None;
        }
        let Some(Validation::Valid(report)) = &self.validation else {
            return None;
        };
        let gpu = entry(report, "gpu");
        let payload = entry(report, "payload");
        let location = field(gpu, "location");
        let location = if location.truthy() {
            py_str(location)
        } else {
            "?".to_string()
        };
        let path = self.path.clone().unwrap_or_else(|| "None".to_string());
        let mut parts = vec![format(
            self.text("confirm_load"),
            &[
                ("index", &py_str(field(gpu, "index"))),
                ("name", &py_str(field(gpu, "name"))),
                ("location", &location),
                ("path", &path),
                ("size", &py_str(field(payload, "size"))),
                ("family", &py_str(field(payload, "family"))),
                ("sha256", &py_str(field(payload, "sha256"))),
                ("steps", &steps(report)),
            ],
        )];
        parts.extend(load_notes(self.language, report));
        parts.push(self.text("applies_on_restart").to_string());
        parts.push(self.text("risk").to_string());
        Some(parts.join("\n\n"))
    }

    /// Ok(question) before Remove override, or Err(error line for Messages) when the dry run is refused.
    pub fn confirm_remove(&self) -> Result<String, String> {
        let selection = Selection {
            instance_id: self.selected.clone(),
            ..Selection::default()
        };
        let report = match unload(&*self.platform, &selection, true) {
            Ok(report) => report,
            Err(error) => {
                return Err(format(
                    self.text("error"),
                    &[("message", &reason(self.language, error.message()))],
                ))
            }
        };
        let gpu = entry(&report, "gpu");
        let location = field(gpu, "location");
        let location = if location.truthy() {
            py_str(location)
        } else {
            "?".to_string()
        };
        let mut parts = vec![format(
            self.text("confirm_remove"),
            &[
                ("index", &py_str(field(gpu, "index"))),
                ("name", &py_str(field(gpu, "name"))),
                ("location", &location),
                ("steps", &steps(&report)),
            ],
        )];
        let message = entry(&report, "message");
        if message.truthy() {
            parts.push(format(self.text("note"), &[("message", &py_str(message))]));
        }
        parts.push(self.text("applies_on_restart").to_string());
        Ok(parts.join("\n\n"))
    }

    /// Mark the controller busy and return the work for a worker thread. The window calls it only when the
    /// action's button is enabled (and after the confirmation for Load and Remove override).
    pub fn start(&mut self, action: Action) -> Job {
        self.busy = true;
        Job {
            action,
            platform: Arc::clone(&self.platform),
            selected: self.selected.clone(),
            data: self.data.clone(),
            language: self.language,
        }
    }

    /// The worker's result arrived: not busy any more. The window then logs the lines, refreshes and shows them.
    pub fn finish(&mut self) {
        self.busy = false;
    }
}

/// Controller._steps: the step lines of a report, indented.
fn steps(report: &Map) -> String {
    let lines: Vec<String> = entry(report, "steps")
        .as_array()
        .map(|steps| steps.iter().map(|step| format!("  {}", step_text(step))).collect())
        .unwrap_or_default();
    lines.join("\n")
}

/// Controller._load_notes: the window's own texts for the notes of a load report.
fn load_notes(language: Language, report: &Map) -> Vec<String> {
    let mut notes = Vec::new();
    if entry(report, "replaces_other_override").truthy() {
        notes.push(text(language, "other_tool").to_string());
    }
    let restores = entry(report, "restores_firmware");
    if restores.truthy() {
        notes.push(format(
            text(language, "firmware_restored"),
            &[("saved", &py_str(restores))],
        ));
    }
    if entry(report, "keeps_gsp_firmware_mode").truthy() {
        notes.push(text(language, "firmware_gsp_kept").to_string());
    }
    notes
}

/// Load, Remove override or Verify on the selected GPU, detached from the controller so it can run in a worker
/// thread: it holds the platform, the selected device instance id, the file data and the language.
pub struct Job {
    action: Action,
    platform: Arc<dyn Platform>,
    selected: Option<String>,
    data: Option<Arc<Vec<u8>>>,
    language: Language,
}

impl Job {
    pub fn action(&self) -> Action {
        self.action
    }

    /// (ok, message lines) of controller.run: never panics (an internal panic becomes the "unexpected" line).
    pub fn run(self) -> (bool, Vec<String>) {
        let language = self.language;
        match catch_unwind(AssertUnwindSafe(|| self.work())) {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => (
                false,
                vec![format(
                    text(language, "error"),
                    &[("message", &reason(language, error.message()))],
                )],
            ),
            Err(payload) => (
                false,
                vec![unexpected(language, &format!("panic: {}", panic_text(&*payload)))],
            ),
        }
    }

    fn text(&self, key: &str) -> &'static str {
        text(self.language, key)
    }

    /// Controller._run.
    fn work(&self) -> Result<(bool, Vec<String>)> {
        let selection = Selection {
            instance_id: self.selected.clone(),
            ..Selection::default()
        };
        let report = match self.action {
            Action::Verify => return self.verify(&selection),
            Action::Load => {
                let Some(data) = &self.data else {
                    // The window offers Load only after a file was read.
                    return Ok((false, vec![unexpected(self.language, "no VBIOS file was read")]));
                };
                load(&*self.platform, data, &selection, false)?
            }
            Action::Unload => unload(&*self.platform, &selection, false)?,
        };
        let index = py_str(field(entry(&report, "gpu"), "index"));
        let mut lines = Vec::new();
        if self.action == Action::Load {
            let payload = entry(&report, "payload");
            lines.push(format(
                self.text("loaded"),
                &[
                    ("index", &index),
                    ("size", &py_str(field(payload, "size"))),
                    ("sha256", &py_str(field(payload, "sha256"))),
                ],
            ));
            lines.extend(load_notes(self.language, &report));
        } else {
            if entry(&report, "steps").truthy() {
                lines.push(format(self.text("removed"), &[("index", &index)]));
            }
            let message = entry(&report, "message");
            if message.truthy() {
                lines.push(format(self.text("note"), &[("message", &py_str(message))]));
            }
        }
        if entry(&report, "next_step").truthy() {
            lines.push(self.text("restart_windows").to_string());
        }
        Ok((true, lines))
    }

    /// Controller._verify.
    fn verify(&self, selection: &Selection) -> Result<(bool, Vec<String>)> {
        let report = verify(&*self.platform, selection)?;
        let readback = entry(&report, "readback");
        let configured = entry(&report, "configured");
        let state = py_str(entry(&report, "state"));
        let prefix = entry(&report, "compared").as_str() == Some("prefix");
        let shown_state = if entry(&report, "match").truthy() && prefix {
            format!("{state}-prefix")
        } else {
            state.clone()
        };
        let compared = py_str(field(readback, "size"));
        let mut lines = vec![format(
            self.text(&format!("verify_{shown_state}")),
            &[
                ("index", &py_str(field(entry(&report, "gpu"), "index"))),
                ("compared", &compared),
            ],
        )];
        if prefix {
            lines.push(format(
                self.text("verify_prefix"),
                &[("compared", &compared), ("size", &py_str(field(configured, "size")))],
            ));
        }
        if let Some(offset) = entry(&report, "first_difference").as_i64() {
            lines.push(format(
                self.text("verify_difference"),
                &[("offset", &format!("{offset:#x}"))],
            ));
        }
        if state == "no-override" && entry(&report, "rom_override_records").truthy() {
            lines.push(self.text("verify_records").to_string());
        }
        lines.push(format(
            self.text("verify_running"),
            &[
                ("size", &py_str(field(readback, "size"))),
                ("sha256", &py_str(field(readback, "sha256"))),
            ],
        ));
        if configured.truthy() {
            lines.push(format(
                self.text("verify_stored"),
                &[
                    ("size", &py_str(field(configured, "size"))),
                    ("sha256", &py_str(field(configured, "sha256"))),
                ],
            ));
        }
        Ok((VERIFY_OK.contains(&state.as_str()), lines))
    }
}

/// The "unexpected" line: Python showed the exception type and text, Rust shows "panic: <message>".
fn unexpected(language: Language, message: &str) -> String {
    format(text(language, "unexpected"), &[("message", message)])
}

// The window moves a Job to a worker thread.
const _: () = {
    const fn send<T: Send>() {}
    send::<Job>();
};

#[cfg(test)]
mod tests;
