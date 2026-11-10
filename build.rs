//! Windows resources for both programs: the version information and the application manifest (res/), compiled
//! with the Windows SDK's rc.exe and linked into each program only.
//!
//! This needs an MSVC target built on Windows. Elsewhere (the checks on Linux) the resources are skipped, with a
//! warning when the target is Windows. VBIOS_LOADER_REQUIRE_RESOURCES=1 (the release build on CI) makes a skipped
//! resource an error, so a release never ships without its manifest.
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};

/// Each program and its FileDescription; its manifest is res/<program>.manifest.
const PROGRAMS: [(&str, &str); 2] = [
    ("vbios-loader", "Windows VBIOS Vix Loader"),
    ("vbios-loader-gui", "Windows VBIOS Vix Loader (window)"),
];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=res");
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=VBIOS_LOADER_REQUIRE_RESOURCES");
    if let Err(message) = run() {
        eprintln!("error: {message}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let required = env::var("VBIOS_LOADER_REQUIRE_RESOURCES").is_ok_and(|value| value == "1");
    let target_windows = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let target_msvc = env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|abi| abi == "msvc");
    // cfg!(windows) is the system the build script runs on, not the target.
    let skipped = if !target_windows {
        Some("the target is not Windows")
    } else if !target_msvc {
        Some("the target is not MSVC")
    } else if !cfg!(windows) {
        Some("rc.exe and link.exe need a build on Windows")
    } else {
        None
    };
    if let Some(reason) = skipped {
        return skip(reason, required, target_windows);
    }
    let Some(rc) = find_rc() else {
        return skip(
            "rc.exe was not found (set RC, or install the Windows SDK)",
            required,
            true,
        );
    };
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?);
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?);
    let res = manifest_dir.join("res");
    let template = read(&res.join("version.rc"))?;
    let version = env::var("CARGO_PKG_VERSION").map_err(|_| "CARGO_PKG_VERSION is not set")?;
    let mut numbers = Vec::new();
    for name in [
        "CARGO_PKG_VERSION_MAJOR",
        "CARGO_PKG_VERSION_MINOR",
        "CARGO_PKG_VERSION_PATCH",
    ] {
        numbers.push(env::var(name).map_err(|_| format!("{name} is not set"))?);
    }
    numbers.push("0".to_string());
    let numbers = numbers.join(",");
    for (program, description) in PROGRAMS {
        let manifest = format!("{program}.manifest");
        fs::copy(res.join(&manifest), out_dir.join(&manifest))
            .map_err(|error| format!("cannot copy res/{manifest}: {error}"))?;
        let script = template
            .replace("@PROGRAM@", &rc_string(program))
            .replace("@FILE_DESCRIPTION@", &rc_string(description))
            .replace("@VERSION_NUMBERS@", &numbers)
            .replace("@VERSION@", &rc_string(&version));
        let script_name = format!("{program}.rc");
        let res_name = format!("{program}.res");
        fs::write(out_dir.join(&script_name), script)
            .map_err(|error| format!("cannot write {script_name}: {error}"))?;
        // rc.exe resolves the manifest file name against its working directory.
        let status = Command::new(&rc)
            .current_dir(&out_dir)
            .args(["/nologo", "/fo", &res_name, &script_name])
            .status()
            .map_err(|error| format!("cannot run {}: {error}", rc.display()))?;
        if !status.success() {
            return Err(format!("{} failed on {script_name} ({status})", rc.display()));
        }
        // link.exe takes a compiled .res file as an input. /MANIFEST:NO keeps the linker from generating a manifest
        // of its own, so the one in the .res file is the only one.
        println!(
            "cargo:rustc-link-arg-bin={program}={}",
            out_dir.join(&res_name).display()
        );
        println!("cargo:rustc-link-arg-bin={program}=/MANIFEST:NO");
    }
    Ok(())
}

/// Skip the resources: an error with VBIOS_LOADER_REQUIRE_RESOURCES=1, a warning when `warn` (a Windows target).
fn skip(reason: &str, required: bool, warn: bool) -> Result<(), String> {
    if required {
        return Err(format!(
            "VBIOS_LOADER_REQUIRE_RESOURCES=1 but the Windows resources cannot be built: {reason}"
        ));
    }
    if warn {
        println!("cargo:warning=Windows resources (version information, manifests) skipped: {reason}");
    }
    Ok(())
}

/// RC if set (a path, or a name on PATH), else the newest Windows Kits 10 x64 rc.exe.
fn find_rc() -> Option<PathBuf> {
    if let Some(rc) = env::var_os("RC").filter(|rc| !rc.is_empty()) {
        return Some(PathBuf::from(rc));
    }
    let program_files = env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
    let mut newest: Option<(Vec<u32>, PathBuf)> = None;
    for entry in fs::read_dir(program_files.join(r"Windows Kits\10\bin")).ok()?.flatten() {
        let Some(version) = entry.file_name().to_str().and_then(kit_version) else {
            continue;
        };
        let rc = entry.path().join(r"x64\rc.exe");
        if rc.is_file() && newest.as_ref().is_none_or(|(best, _)| version > *best) {
            newest = Some((version, rc));
        }
    }
    newest.map(|(_, rc)| rc)
}

/// "10.0.22621.0" -> [10, 0, 22621, 0]; other folder names (such as x64) -> None.
fn kit_version(name: &str) -> Option<Vec<u32>> {
    let parts = name
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<Vec<u32>>>()?;
    (parts.len() == 4).then_some(parts)
}

/// A string for a quoted rc.exe literal: a double quote is written twice.
fn rc_string(text: &str) -> String {
    text.replace('"', "\"\"")
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("cannot read {}: {error}", path.display()))
}
