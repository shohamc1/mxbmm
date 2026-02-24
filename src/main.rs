use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use walkdir::WalkDir;
use zip::ZipArchive;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModType {
    Track,
    Bike,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BikeCategory {
    Motocross,
    Supercross,
}

impl BikeCategory {
    fn folder_name(self) -> &'static str {
        match self {
            Self::Motocross => "motocross",
            Self::Supercross => "supercross",
        }
    }
}

#[derive(Clone)]
struct ModEntry {
    name: String,
    path: PathBuf,
}

#[derive(Clone, Copy)]
enum StatusKind {
    Info,
    Success,
    Error,
}

struct StatusMessage {
    kind: StatusKind,
    text: String,
}

struct PendingInstall {
    archive_path: PathBuf,
    temp_extract_dir: PathBuf,
    mod_type: ModType,
    bike_category: BikeCategory,
    custom_name: String,
    notes: String,
    version: String,
}

struct MxbmmApp {
    mods_root_input: String,
    status: Option<StatusMessage>,
    tracks: Vec<ModEntry>,
    bikes_motocross: Vec<ModEntry>,
    bikes_supercross: Vec<ModEntry>,
    pending_install: Option<PendingInstall>,
    pending_uninstall: Option<ModEntry>,
}

impl Default for MxbmmApp {
    fn default() -> Self {
        let mods_root = default_mods_root();
        let mut app = Self {
            mods_root_input: mods_root.to_string_lossy().to_string(),
            status: None,
            tracks: Vec::new(),
            bikes_motocross: Vec::new(),
            bikes_supercross: Vec::new(),
            pending_install: None,
            pending_uninstall: None,
        };
        app.refresh_mod_lists();
        app
    }
}

impl MxbmmApp {
    fn set_status(&mut self, kind: StatusKind, text: impl Into<String>) {
        self.status = Some(StatusMessage {
            kind,
            text: text.into(),
        });
    }

    fn mods_root(&self) -> PathBuf {
        PathBuf::from(self.mods_root_input.trim())
    }

    fn tracks_dir(&self) -> PathBuf {
        self.mods_root().join("tracks")
    }

    fn bikes_dir(&self) -> PathBuf {
        self.mods_root().join("bikes")
    }

    fn bikes_category_dir(&self, category: BikeCategory) -> PathBuf {
        self.bikes_dir().join(category.folder_name())
    }

    fn refresh_mod_lists(&mut self) {
        self.tracks = read_mod_entries(&self.tracks_dir());
        self.bikes_motocross = read_mod_entries(&self.bikes_category_dir(BikeCategory::Motocross));
        self.bikes_supercross =
            read_mod_entries(&self.bikes_category_dir(BikeCategory::Supercross));
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped_files.is_empty() {
            return;
        }

        if self.pending_install.is_some() {
            self.set_status(
                StatusKind::Info,
                "Finish or cancel the current pending install before dropping another archive.",
            );
            return;
        }

        let files: Vec<PathBuf> = dropped_files.into_iter().filter_map(|f| f.path).collect();
        if files.len() != 1 {
            self.set_status(
                StatusKind::Error,
                "Drop exactly one archive file at a time.",
            );
            return;
        }

        let archive_path = files[0].clone();
        if !is_supported_archive(&archive_path) {
            self.set_status(
                StatusKind::Error,
                "Unsupported archive type. Currently only .zip is supported.",
            );
            return;
        }

        match self.prepare_pending_install(archive_path.clone()) {
            Ok(pending) => {
                self.pending_install = Some(pending);
                self.set_status(
                    StatusKind::Info,
                    format!(
                        "Archive extracted: {}. Fill out mod details and install.",
                        archive_path.display()
                    ),
                );
            }
            Err(err) => {
                self.set_status(
                    StatusKind::Error,
                    format!(
                        "Failed to extract archive {}: {}",
                        archive_path.display(),
                        err
                    ),
                );
            }
        }
    }

    fn prepare_pending_install(&self, archive_path: PathBuf) -> Result<PendingInstall, String> {
        let temp_extract_dir = create_temp_extract_dir().map_err(|e| e.to_string())?;
        if let Err(err) = extract_zip_archive(&archive_path, &temp_extract_dir) {
            let _ = fs::remove_dir_all(&temp_extract_dir);
            return Err(err.to_string());
        }

        let default_name = guess_mod_name(&temp_extract_dir, &archive_path);
        Ok(PendingInstall {
            archive_path,
            temp_extract_dir,
            mod_type: ModType::Track,
            bike_category: BikeCategory::Motocross,
            custom_name: default_name,
            notes: String::new(),
            version: String::new(),
        })
    }

    fn install_pending(&mut self) {
        let Some(pending) = self.pending_install.take() else {
            return;
        };

        let install_name = pending.custom_name.trim().to_string();
        if install_name.is_empty() {
            self.set_status(StatusKind::Error, "Install name cannot be empty.");
            self.pending_install = Some(pending);
            return;
        }

        let base_destination = match pending.mod_type {
            ModType::Track => self.tracks_dir(),
            ModType::Bike => self.bikes_category_dir(pending.bike_category),
        };

        if let Err(err) = fs::create_dir_all(&base_destination) {
            self.set_status(
                StatusKind::Error,
                format!(
                    "Failed to create destination directory {}: {}",
                    base_destination.display(),
                    err
                ),
            );
            self.pending_install = Some(pending);
            return;
        }

        let destination = base_destination.join(&install_name);
        if destination.exists() {
            self.set_status(
                StatusKind::Error,
                format!(
                    "Destination already exists: {}. Choose another install name.",
                    destination.display()
                ),
            );
            self.pending_install = Some(pending);
            return;
        }

        if let Err(err) = fs::create_dir_all(&destination) {
            self.set_status(
                StatusKind::Error,
                format!(
                    "Failed to create install folder {}: {}",
                    destination.display(),
                    err
                ),
            );
            self.pending_install = Some(pending);
            return;
        }

        let source_root = pick_source_root(&pending.temp_extract_dir);
        if let Err(err) = copy_dir_contents(&source_root, &destination) {
            let _ = fs::remove_dir_all(&destination);
            self.set_status(
                StatusKind::Error,
                format!("Install failed while copying files: {}", err),
            );
            self.pending_install = Some(pending);
            return;
        }

        if let Err(err) = write_metadata_file(
            &destination,
            pending.mod_type,
            pending.bike_category,
            &pending.version,
            &pending.notes,
            &pending.archive_path,
        ) {
            self.set_status(
                StatusKind::Info,
                format!(
                    "Installed, but failed to write metadata file in {}: {}",
                    destination.display(),
                    err
                ),
            );
        } else {
            self.set_status(
                StatusKind::Success,
                format!("Installed mod to {}", destination.display()),
            );
        }

        let _ = fs::remove_dir_all(&pending.temp_extract_dir);
        self.refresh_mod_lists();
    }

    fn uninstall_mod(&mut self, entry: &ModEntry) {
        let result = if entry.path.is_dir() {
            fs::remove_dir_all(&entry.path)
        } else {
            fs::remove_file(&entry.path)
        };

        match result {
            Ok(()) => {
                self.set_status(StatusKind::Success, format!("Removed mod {}", entry.name));
                self.refresh_mod_lists();
            }
            Err(err) => {
                self.set_status(
                    StatusKind::Error,
                    format!("Failed to remove {}: {}", entry.path.display(), err),
                );
            }
        }
    }

    fn draw_status(&self, ui: &mut egui::Ui) {
        let Some(status) = &self.status else {
            return;
        };

        let color = match status.kind {
            StatusKind::Info => egui::Color32::LIGHT_BLUE,
            StatusKind::Success => egui::Color32::LIGHT_GREEN,
            StatusKind::Error => egui::Color32::LIGHT_RED,
        };

        ui.colored_label(color, &status.text);
    }

    fn draw_pending_install_ui(&mut self, ui: &mut egui::Ui) {
        if self.pending_install.is_none() {
            return;
        }

        let mut clicked_install = false;
        let mut clicked_cancel = false;

        {
            let pending = self.pending_install.as_mut().expect("checked above");
            ui.separator();
            ui.heading("Pending Install");
            ui.label(format!("Archive: {}", pending.archive_path.display()));

            egui::ComboBox::from_label("Mod type")
                .selected_text(match pending.mod_type {
                    ModType::Track => "Track",
                    ModType::Bike => "Bike",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut pending.mod_type, ModType::Track, "Track");
                    ui.selectable_value(&mut pending.mod_type, ModType::Bike, "Bike");
                });

            if pending.mod_type == ModType::Bike {
                egui::ComboBox::from_label("Bike category")
                    .selected_text(match pending.bike_category {
                        BikeCategory::Motocross => "Motocross",
                        BikeCategory::Supercross => "Supercross",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut pending.bike_category,
                            BikeCategory::Motocross,
                            "Motocross",
                        );
                        ui.selectable_value(
                            &mut pending.bike_category,
                            BikeCategory::Supercross,
                            "Supercross",
                        );
                    });
            }

            ui.label("Install name");
            ui.text_edit_singleline(&mut pending.custom_name);

            ui.label("Version (optional)");
            ui.text_edit_singleline(&mut pending.version);

            ui.label("Notes (optional)");
            ui.text_edit_multiline(&mut pending.notes);

            ui.horizontal(|ui| {
                if ui.button("Install").clicked() {
                    clicked_install = true;
                }

                if ui.button("Cancel").clicked() {
                    clicked_cancel = true;
                }
            });
        }

        if clicked_install {
            self.install_pending();
        }

        if clicked_cancel {
            if let Some(pending) = self.pending_install.take() {
                let _ = fs::remove_dir_all(&pending.temp_extract_dir);
            }
            self.set_status(StatusKind::Info, "Pending install canceled.");
        }
    }

    fn draw_mod_list(ui: &mut egui::Ui, title: &str, mods: &[ModEntry]) -> Option<ModEntry> {
        ui.heading(title);
        if mods.is_empty() {
            ui.label("No mods found.");
            return None;
        }

        let mut uninstall_target = None;
        egui::ScrollArea::vertical()
            .max_height(180.0)
            .show(ui, |ui| {
                for entry in mods {
                    ui.horizontal(|ui| {
                        ui.label(&entry.name);
                        if ui.button("Uninstall").clicked() {
                            uninstall_target = Some(entry.clone());
                        }
                    });
                }
            });
        uninstall_target
    }
}

impl eframe::App for MxbmmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("MX Bikes Mod Manager");
            ui.label("Drag and drop a .zip archive to begin installing a mod.");

            let hovered_files = ctx.input(|i| i.raw.hovered_files.clone());
            if !hovered_files.is_empty() {
                ui.colored_label(
                    egui::Color32::LIGHT_YELLOW,
                    "Drop archive now to extract and configure install details.",
                );
            }

            ui.separator();
            ui.label("Mods root path");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.mods_root_input);
                if ui.button("Refresh").clicked() {
                    self.refresh_mod_lists();
                    self.set_status(StatusKind::Info, "Refreshed installed mod list.");
                }
            });
            self.draw_status(ui);

            self.draw_pending_install_ui(ui);

            ui.separator();
            ui.heading("Installed Mods");
            ui.columns(3, |cols| {
                let tracks_uninstall = Self::draw_mod_list(&mut cols[0], "Tracks", &self.tracks);
                let mx_uninstall =
                    Self::draw_mod_list(&mut cols[1], "Bikes (Motocross)", &self.bikes_motocross);
                let sx_uninstall =
                    Self::draw_mod_list(&mut cols[2], "Bikes (Supercross)", &self.bikes_supercross);

                self.pending_uninstall = tracks_uninstall
                    .or(mx_uninstall)
                    .or(sx_uninstall)
                    .or(self.pending_uninstall.clone());
            });
        });

        if let Some(target) = self.pending_uninstall.clone() {
            let mut keep_open = true;
            egui::Window::new("Confirm uninstall")
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(format!("Remove '{}' ?", target.name));
                    ui.label(target.path.display().to_string());

                    ui.horizontal(|ui| {
                        if ui.button("Delete").clicked() {
                            self.uninstall_mod(&target);
                            keep_open = false;
                        }

                        if ui.button("Cancel").clicked() {
                            keep_open = false;
                        }
                    });
                });

            if !keep_open {
                self.pending_uninstall = None;
            }
        }
    }
}

fn default_mods_root() -> PathBuf {
    if let Ok(path) = std::env::var("MXBMM_MODS_ROOT") {
        return PathBuf::from(path);
    }

    if let Some(documents) = dirs::document_dir() {
        return documents.join("PiBoSo").join("MX Bikes").join("mods");
    }

    PathBuf::from(".").join("mods")
}

fn read_mod_entries(dir: &Path) -> Vec<ModEntry> {
    let mut entries = Vec::new();
    let read_dir = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return entries,
    };

    for item in read_dir.flatten() {
        let path = item.path();
        let name = item.file_name().to_string_lossy().to_string();
        entries.push(ModEntry { name, path });
    }

    entries.sort_by_key(|e| e.name.to_lowercase());
    entries
}

fn is_supported_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("zip"))
        .unwrap_or(false)
}

fn extract_zip_archive(archive_path: &Path, destination: &Path) -> io::Result<()> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        let Some(enclosed_name) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };

        let outpath = destination.join(enclosed_name);
        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = File::create(&outpath)?;
        io::copy(&mut entry, &mut output)?;
    }

    Ok(())
}

fn guess_mod_name(extract_dir: &Path, archive_path: &Path) -> String {
    let mut entries = match fs::read_dir(extract_dir) {
        Ok(read_dir) => read_dir.flatten().collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };

    if entries.len() == 1 {
        let first = entries.remove(0);
        if first.path().is_dir() {
            return first.file_name().to_string_lossy().to_string();
        }
    }

    archive_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mod")
        .to_string()
}

fn pick_source_root(extract_dir: &Path) -> PathBuf {
    let mut entries = match fs::read_dir(extract_dir) {
        Ok(read_dir) => read_dir.flatten().collect::<Vec<_>>(),
        Err(_) => return extract_dir.to_path_buf(),
    };

    if entries.len() == 1 {
        let entry = entries.remove(0);
        let path = entry.path();
        if path.is_dir() {
            return path;
        }
    }

    extract_dir.to_path_buf()
}

fn copy_dir_contents(source: &Path, destination: &Path) -> io::Result<()> {
    for entry in WalkDir::new(source) {
        let entry = entry.map_err(|err| io::Error::new(io::ErrorKind::Other, err.to_string()))?;
        let path = entry.path();
        let rel = match path.strip_prefix(source) {
            Ok(r) if !r.as_os_str().is_empty() => r,
            _ => continue,
        };

        let target = destination.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }

        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        if entry.file_type().is_file() {
            fs::copy(path, &target)?;
        }
    }

    Ok(())
}

fn write_metadata_file(
    destination: &Path,
    mod_type: ModType,
    bike_category: BikeCategory,
    version: &str,
    notes: &str,
    archive_path: &Path,
) -> io::Result<()> {
    let mut file = File::create(destination.join("_mxbmm_meta.txt"))?;
    writeln!(
        file,
        "mod_type={}",
        match mod_type {
            ModType::Track => "track",
            ModType::Bike => "bike",
        }
    )?;
    writeln!(file, "bike_category={}", bike_category.folder_name())?;
    writeln!(file, "version={}", version.trim())?;
    writeln!(file, "archive={}", archive_path.display())?;
    writeln!(file, "notes={}", notes.replace('\n', "\\n"))?;
    Ok(())
}

fn create_temp_extract_dir() -> io::Result<PathBuf> {
    let base = std::env::temp_dir().join("mxbmm_extracts");
    fs::create_dir_all(&base)?;

    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();

    for attempt in 0..1000_u32 {
        let dir = base.join(format!("extract-{}-{}-{}", pid, now_nanos, attempt));
        if !dir.exists() {
            fs::create_dir_all(&dir)?;
            return Ok(dir);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Failed to create a unique extraction directory.",
    ))
}

fn main() -> eframe::Result<()> {
    configure_wgpu_backend_env();

    match run_with_renderer(eframe::Renderer::Wgpu) {
        Ok(()) => Ok(()),
        Err(wgpu_err) => {
            eprintln!("WGPU startup failed: {wgpu_err}");
            eprintln!("Falling back to OpenGL renderer...");

            match run_with_renderer(eframe::Renderer::Glow) {
                Ok(()) => Ok(()),
                Err(glow_err) => {
                    eprintln!("OpenGL startup failed: {glow_err}");
                    Err(glow_err)
                }
            }
        }
    }
}

fn run_with_renderer(renderer: eframe::Renderer) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        renderer,
        ..Default::default()
    };
    eframe::run_native(
        "MX Bikes Mod Manager",
        options,
        Box::new(|_cc| Ok(Box::new(MxbmmApp::default()))),
    )
}

fn configure_wgpu_backend_env() {
    if std::env::var_os("WGPU_BACKEND").is_none() {
        // Some Windows VMs only expose DX11 adapters.
        std::env::set_var("WGPU_BACKEND", "dx12,dx11,vulkan,metal,gl");
    }
}
