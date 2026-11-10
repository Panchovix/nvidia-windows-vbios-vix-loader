//! Windows VBIOS Vix Loader: load a modified VBIOS on Windows through the NVIDIA driver's registry override,
//! without flashing it.
//!
//! The library holds everything; the two programs in `src/bin` only call [`cli::main`] and [`gui::main`].
//! Everything except `win32`, the window in `gui` and the DLL loading in `nvapi` is plain Rust that builds and
//! is tested on any system: the registry, SetupAPI and NvAPI sit behind the [`platform::Platform`] trait, and
//! the tests use a fake.

pub mod cli;
pub mod controller;
pub mod devices;
pub mod error;
pub mod families;
pub mod gui;
pub mod image;
pub mod json;
pub mod nvapi;
pub mod operations;
pub mod payload;
pub mod platform;
pub mod pyfmt;
pub mod sha256;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(windows)]
pub mod win32;

/// The version of both programs (Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
