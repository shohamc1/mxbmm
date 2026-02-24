use std::fs;
use std::path::PathBuf;

use eframe::egui;

use crate::fs_ops::{
    copy_dir_contents, create_fs_watcher, create_temp_extract_dir, extract_zip_archive,
    guess_mod_name, is_pkz_file, is_pnt_file, is_supported_archive, pick_source_root,
    read_mod_entries, with_extension_if_missing, write_metadata_file,
};
use crate::model::{
    FsWatcherState, InstallTarget, ModEntry, PendingInstall, PendingSource, StatusKind,
    StatusMessage, ALL_INSTALL_TARGETS,
};

pub struct MxbmmApp {
    mods_root_input: String,
    status: Option<StatusMessage>,
    tracks: Vec<ModEntry>,
    bikes_motocross: Vec<ModEntry>,
    bikes_supercross: Vec<ModEntry>,
    bikes_paints: Vec<ModEntry>,
    tyres: Vec<ModEntry>,
    rider_models: Vec<ModEntry>,
    rider_paints: Vec<ModEntry>,
    rider_gloves: Vec<ModEntry>,
    rider_helmets: Vec<ModEntry>,
    rider_helmet_paints: Vec<ModEntry>,
    rider_boots: Vec<ModEntry>,
    rider_boot_paints: Vec<ModEntry>,
    rider_protections: Vec<ModEntry>,
    pending_install: Option<PendingInstall>,
    pending_uninstall: Option<ModEntry>,
    fs_watcher: Option<FsWatcherState>,
    watcher_error_for_root: Option<PathBuf>,
}

impl Default for MxbmmApp {
    fn default() -> Self {
        let mods_root = crate::fs_ops::default_mods_root();
        let mut app = Self {
            mods_root_input: mods_root.to_string_lossy().to_string(),
            status: None,
            tracks: Vec::new(),
            bikes_motocross: Vec::new(),
            bikes_supercross: Vec::new(),
            bikes_paints: Vec::new(),
            tyres: Vec::new(),
            rider_models: Vec::new(),
            rider_paints: Vec::new(),
            rider_gloves: Vec::new(),
            rider_helmets: Vec::new(),
            rider_helmet_paints: Vec::new(),
            rider_boots: Vec::new(),
            rider_boot_paints: Vec::new(),
            rider_protections: Vec::new(),
            pending_install: None,
            pending_uninstall: None,
            fs_watcher: None,
            watcher_error_for_root: None,
        };
        app.refresh_mod_lists();
        app.sync_fs_watcher();
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

    fn target_dir(&self, target: InstallTarget) -> PathBuf {
        self.mods_root().join(target.relative_path())
    }

    fn refresh_mod_lists(&mut self) {
        self.tracks = read_mod_entries(&self.target_dir(InstallTarget::Tracks), &[]);
        self.bikes_motocross =
            read_mod_entries(&self.target_dir(InstallTarget::BikesMotocross), &[]);
        self.bikes_supercross =
            read_mod_entries(&self.target_dir(InstallTarget::BikesSupercross), &[]);
        self.bikes_paints = read_mod_entries(&self.target_dir(InstallTarget::BikesPaints), &[]);
        self.tyres = read_mod_entries(&self.target_dir(InstallTarget::Tyres), &[]);
        self.rider_models = read_mod_entries(
            &self.target_dir(InstallTarget::RiderModels),
            &["paints", "gloves"],
        );
        self.rider_paints = read_mod_entries(&self.target_dir(InstallTarget::RiderPaints), &[]);
        self.rider_gloves = read_mod_entries(&self.target_dir(InstallTarget::RiderGloves), &[]);
        self.rider_helmets =
            read_mod_entries(&self.target_dir(InstallTarget::RiderHelmets), &["paints"]);
        self.rider_helmet_paints =
            read_mod_entries(&self.target_dir(InstallTarget::RiderHelmetPaints), &[]);
        self.rider_boots =
            read_mod_entries(&self.target_dir(InstallTarget::RiderBoots), &["paints"]);
        self.rider_boot_paints =
            read_mod_entries(&self.target_dir(InstallTarget::RiderBootPaints), &[]);
        self.rider_protections =
            read_mod_entries(&self.target_dir(InstallTarget::RiderProtections), &[]);
    }

    fn sync_fs_watcher(&mut self) {
        let root = self.mods_root();
        if self
            .fs_watcher
            .as_ref()
            .map(|w| w.root == root)
            .unwrap_or(false)
        {
            return;
        }

        self.fs_watcher = None;
        if !root.exists() {
            return;
        }

        match create_fs_watcher(&root) {
            Ok(watcher) => {
                self.fs_watcher = Some(watcher);
                self.watcher_error_for_root = None;
            }
            Err(err) => {
                if self.watcher_error_for_root.as_ref() != Some(&root) {
                    self.set_status(
                        StatusKind::Info,
                        format!(
                            "File watcher unavailable for {}: {}. Use Refresh manually.",
                            root.display(),
                            err
                        ),
                    );
                    self.watcher_error_for_root = Some(root);
                }
            }
        }
    }

    fn process_fs_events(&mut self) {
        let mut should_refresh = false;
        let mut event_error: Option<String> = None;
        if let Some(watcher) = &self.fs_watcher {
            while let Ok(event_result) = watcher.rx.try_recv() {
                match event_result {
                    Ok(_event) => {
                        should_refresh = true;
                    }
                    Err(err) => {
                        event_error = Some(err.to_string());
                    }
                }
            }
        }

        if should_refresh {
            self.refresh_mod_lists();
        }
        if let Some(err) = event_error {
            self.set_status(
                StatusKind::Info,
                format!("File watcher event error: {}. Refresh may be needed.", err),
            );
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped_files = ctx.input(|i| i.raw.dropped_files.clone());
        if dropped_files.is_empty() {
            return;
        }

        if self.pending_install.is_some() {
            self.set_status(
                StatusKind::Info,
                "Finish or cancel the current pending install before dropping another file.",
            );
            return;
        }

        let files: Vec<PathBuf> = dropped_files.into_iter().filter_map(|f| f.path).collect();
        if files.len() != 1 {
            self.set_status(StatusKind::Error, "Drop exactly one file at a time.");
            return;
        }

        let file_path = files[0].clone();
        if is_pkz_file(&file_path) {
            match self.prepare_pending_pkz_install(file_path.clone()) {
                Ok(pending) => {
                    self.pending_install = Some(pending);
                    self.set_status(
                        StatusKind::Info,
                        format!(
                            ".pkz file loaded: {}. Select mod type and install.",
                            file_path.display()
                        ),
                    );
                }
                Err(err) => {
                    self.set_status(
                        StatusKind::Error,
                        format!(
                            "Failed to prepare .pkz file {}: {}",
                            file_path.display(),
                            err
                        ),
                    );
                }
            }
            return;
        }

        if is_pnt_file(&file_path) {
            match self.prepare_pending_pnt_install(file_path.clone()) {
                Ok(pending) => {
                    self.pending_install = Some(pending);
                    self.set_status(
                        StatusKind::Info,
                        format!(
                            ".pnt file loaded: {}. Default target is Rider Paints; change it if needed.",
                            file_path.display()
                        ),
                    );
                }
                Err(err) => {
                    self.set_status(
                        StatusKind::Error,
                        format!(
                            "Failed to prepare .pnt file {}: {}",
                            file_path.display(),
                            err
                        ),
                    );
                }
            }
            return;
        }

        if !is_supported_archive(&file_path) {
            self.set_status(
                StatusKind::Error,
                "Unsupported file type. Supported: .zip, .pkz, and .pnt.",
            );
            return;
        }

        match self.prepare_pending_install(file_path.clone()) {
            Ok(pending) => {
                self.pending_install = Some(pending);
                self.set_status(
                    StatusKind::Info,
                    format!(
                        "Archive extracted: {}. Fill out mod details and install.",
                        file_path.display()
                    ),
                );
            }
            Err(err) => {
                self.set_status(
                    StatusKind::Error,
                    format!("Failed to extract archive {}: {}", file_path.display(), err),
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
            source: PendingSource::Zip {
                archive_path,
                temp_extract_dir,
            },
            install_target: InstallTarget::Tracks,
            custom_name: default_name,
            notes: String::new(),
            version: String::new(),
        })
    }

    fn prepare_pending_pkz_install(&self, pkz_path: PathBuf) -> Result<PendingInstall, String> {
        if !pkz_path.exists() {
            return Err("File does not exist.".to_string());
        }

        let default_name = pkz_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("track")
            .to_string();

        Ok(PendingInstall {
            source: PendingSource::Pkz { pkz_path },
            install_target: InstallTarget::Tracks,
            custom_name: default_name,
            notes: String::new(),
            version: String::new(),
        })
    }

    fn prepare_pending_pnt_install(&self, pnt_path: PathBuf) -> Result<PendingInstall, String> {
        if !pnt_path.exists() {
            return Err("File does not exist.".to_string());
        }

        let default_name = pnt_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("rider")
            .to_string();

        Ok(PendingInstall {
            source: PendingSource::Pnt { pnt_path },
            install_target: InstallTarget::RiderPaints,
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

        let base_destination = self.target_dir(pending.install_target);

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

        match pending.source.clone() {
            PendingSource::Zip {
                archive_path,
                temp_extract_dir,
            } => {
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

                let source_root = pick_source_root(&temp_extract_dir);
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
                    pending.install_target,
                    &pending.version,
                    &pending.notes,
                    &archive_path,
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
            }
            PendingSource::Pkz { pkz_path } => {
                let file_name = with_extension_if_missing(&install_name, ".pkz");
                let destination = base_destination.join(file_name);
                if destination.exists() {
                    self.set_status(
                        StatusKind::Error,
                        format!("Destination already exists: {}.", destination.display()),
                    );
                    self.pending_install = Some(pending);
                    return;
                }

                if let Err(err) = fs::copy(pkz_path, &destination) {
                    self.set_status(
                        StatusKind::Error,
                        format!(
                            "Failed to install .pkz file to {}: {}",
                            destination.display(),
                            err
                        ),
                    );
                    self.pending_install = Some(pending);
                    return;
                }

                self.set_status(
                    StatusKind::Success,
                    format!("Installed mod file to {}", destination.display()),
                );
            }
            PendingSource::Pnt { pnt_path } => {
                let file_name = with_extension_if_missing(&install_name, ".pnt");
                let destination = base_destination.join(file_name);
                if destination.exists() {
                    self.set_status(
                        StatusKind::Error,
                        format!("Destination already exists: {}.", destination.display()),
                    );
                    self.pending_install = Some(pending);
                    return;
                }

                if let Err(err) = fs::copy(pnt_path, &destination) {
                    self.set_status(
                        StatusKind::Error,
                        format!(
                            "Failed to install .pnt file to {}: {}",
                            destination.display(),
                            err
                        ),
                    );
                    self.pending_install = Some(pending);
                    return;
                }

                self.set_status(
                    StatusKind::Success,
                    format!("Installed mod file to {}", destination.display()),
                );
            }
        }

        pending.source.cleanup();
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
            ui.label(format!("File: {}", pending.source.input_path().display()));

            egui::ComboBox::from_label("Install location")
                .selected_text(pending.install_target.label())
                .show_ui(ui, |ui| {
                    for target in ALL_INSTALL_TARGETS {
                        ui.selectable_value(&mut pending.install_target, target, target.label());
                    }
                });

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
                pending.source.cleanup();
            }
            self.set_status(StatusKind::Info, "Pending install canceled.");
        }
    }

    fn draw_mod_list(ui: &mut egui::Ui, title: &str, mods: &[ModEntry]) -> Option<ModEntry> {
        let mut uninstall_target = None;
        egui::CollapsingHeader::new(format!("{title} ({})", mods.len()))
            .default_open(false)
            .show(ui, |ui| {
                if mods.is_empty() {
                    ui.label("No mods found.");
                    return;
                }

                egui::ScrollArea::vertical()
                    .id_salt(format!("mod_list_scroll_{}", title))
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
            });
        uninstall_target
    }
}

impl eframe::App for MxbmmApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_fs_watcher();
        self.process_fs_events();
        self.handle_dropped_files(ctx);

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("MX Bikes Mod Manager");
            ui.label("Drag and drop a .zip archive, .pkz file, or .pnt file to install.");

            let hovered_files = ctx.input(|i| i.raw.hovered_files.clone());
            if !hovered_files.is_empty() {
                ui.colored_label(
                    egui::Color32::LIGHT_YELLOW,
                    "Drop file now to configure install details.",
                );
            }

            ui.separator();
            ui.label("Mods root path");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut self.mods_root_input);
                if ui.button("Refresh").clicked() {
                    self.refresh_mod_lists();
                    self.sync_fs_watcher();
                    self.set_status(StatusKind::Info, "Refreshed installed mod list.");
                }
            });
            self.draw_status(ui);

            self.draw_pending_install_ui(ui);

            ui.separator();
            ui.heading("Installed Mods");
            let uninstall_target = egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    None.or(Self::draw_mod_list(ui, "Tracks", &self.tracks))
                        .or(Self::draw_mod_list(
                            ui,
                            "Bikes (Motocross)",
                            &self.bikes_motocross,
                        ))
                        .or(Self::draw_mod_list(
                            ui,
                            "Bikes (Supercross)",
                            &self.bikes_supercross,
                        ))
                        .or(Self::draw_mod_list(ui, "Bike Paints", &self.bikes_paints))
                        .or(Self::draw_mod_list(ui, "Tyres/Wheels", &self.tyres))
                        .or(Self::draw_mod_list(ui, "Rider Models", &self.rider_models))
                        .or(Self::draw_mod_list(ui, "Rider Paints", &self.rider_paints))
                        .or(Self::draw_mod_list(ui, "Rider Gloves", &self.rider_gloves))
                        .or(Self::draw_mod_list(
                            ui,
                            "Helmet Models",
                            &self.rider_helmets,
                        ))
                        .or(Self::draw_mod_list(
                            ui,
                            "Helmet Paints",
                            &self.rider_helmet_paints,
                        ))
                        .or(Self::draw_mod_list(ui, "Boot Models", &self.rider_boots))
                        .or(Self::draw_mod_list(
                            ui,
                            "Boot Paints",
                            &self.rider_boot_paints,
                        ))
                        .or(Self::draw_mod_list(
                            ui,
                            "Protections",
                            &self.rider_protections,
                        ))
                })
                .inner;
            self.pending_uninstall = uninstall_target.or(self.pending_uninstall.clone());
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
