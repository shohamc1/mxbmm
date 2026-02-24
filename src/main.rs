#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod fs_ops;
mod model;

use app::MxbmmApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "MX Bikes Mod Manager",
        options,
        Box::new(|_cc| Ok(Box::new(MxbmmApp::default()))),
    )
}
