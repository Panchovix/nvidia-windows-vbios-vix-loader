//! vbios-loader-gui.exe: the window. No console of its own (it attaches to the parent's for --help and errors).
#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    std::process::exit(vbios_loader::gui::main());
}
