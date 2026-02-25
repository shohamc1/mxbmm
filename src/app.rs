use std::collections::HashMap;
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
    mod_lists: HashMap<InstallTarget, Vec<ModEntry>>,
    pending_install: Option<PendingInstall>,
    pending_uninstall: Option<ModEntry>,
    last_install_target: InstallTarget,
    fs_watcher: Option<FsWatcherState>,
    watcher_error_for_root: Option<PathBuf>,
}

impl Default for MxbmmApp {
    fn default() -> Self {
        let mods_root = crate::fs_ops::default_mods_root();
        let mut app = Self {
            mods_root_input: mods_root.to_string_lossy().to_string(),
            status: None,
            mod_lists: HashMap::new(),
            pending_install: None,
            pending_uninstall: None,
            last_install_target: InstallTarget::Tracks,
            fs_watcher: None,
            watcher_error_for_root: None,
        };
        app.refresh_mod_lists();
        app.sync_fs_watcher();
        app
    }
}

impl Drop for MxbmmApp {
    fn drop(&mut self) {
        if let Some(pending) = self.pending_install.take() {
            pending.source.cleanup();
        }
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
        for &target in &ALL_INSTALL_TARGETS {
            let entries =
                read_mod_entries(&self.target_dir(target), target.excluded_subdirs());
            self.mod_lists.insert(target, entries);
        }
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
            match self.prepare_pending_single_file_install(
                file_path.clone(),
                self.last_install_target,
                "track",
                |p| PendingSource::Pkz { pkz_path: p },
            ) {
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
            match self.prepare_pending_single_file_install(
                file_path.clone(),
                self.last_install_target,
                "rider",
                |p| PendingSource::Pnt { pnt_path: p },
            ) {
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

        match self.prepare_pending_zip_install(file_path.clone()) {
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

    fn prepare_pending_zip_install(
        &self,
        archive_path: PathBuf,
    ) -> Result<PendingInstall, String> {
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
            install_target: self.last_install_target,
            custom_name: default_name,
            notes: String::new(),
            version: String::new(),
        })
    }

    fn prepare_pending_single_file_install(
        &self,
        path: PathBuf,
        default_target: InstallTarget,
        fallback_name: &str,
        make_source: impl FnOnce(PathBuf) -> PendingSource,
    ) -> Result<PendingInstall, String> {
        if !path.exists() {
            return Err("File does not exist.".to_string());
        }

        let default_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(fallback_name)
            .to_string();

        Ok(PendingInstall {
            source: make_source(path),
            install_target: default_target,
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

        let result: Result<(StatusKind, String), String> = match &pending.source {
            PendingSource::Zip {
                archive_path,
                temp_extract_dir,
            } => {
                let destination = base_destination.join(&install_name);
                if destination.exists() {
                    Err(format!(
                        "Destination already exists: {}. Choose another install name.",
                        destination.display()
                    ))
                } else if let Err(err) = fs::create_dir_all(&destination) {
                    Err(format!(
                        "Failed to create install folder {}: {}",
                        destination.display(),
                        err
                    ))
                } else {
                    let source_root = pick_source_root(temp_extract_dir);
                    if let Err(err) = copy_dir_contents(&source_root, &destination) {
                        let _ = fs::remove_dir_all(&destination);
                        Err(format!("Install failed while copying files: {}", err))
                    } else if let Err(err) = write_metadata_file(
                        &destination,
                        pending.install_target,
                        &pending.version,
                        &pending.notes,
                        archive_path,
                    ) {
                        Ok((
                            StatusKind::Info,
                            format!(
                                "Installed, but failed to write metadata file in {}: {}",
                                destination.display(),
                                err
                            ),
                        ))
                    } else {
                        Ok((
                            StatusKind::Success,
                            format!("Installed mod to {}", destination.display()),
                        ))
                    }
                }
            }
            PendingSource::Pkz { pkz_path } => {
                let file_name = with_extension_if_missing(&install_name, ".pkz");
                let destination = base_destination.join(file_name);
                if destination.exists() {
                    Err(format!(
                        "Destination already exists: {}.",
                        destination.display()
                    ))
                } else if let Err(err) = fs::copy(pkz_path, &destination) {
                    Err(format!(
                        "Failed to install .pkz file to {}: {}",
                        destination.display(),
                        err
                    ))
                } else {
                    Ok((
                        StatusKind::Success,
                        format!("Installed mod file to {}", destination.display()),
                    ))
                }
            }
            PendingSource::Pnt { pnt_path } => {
                let file_name = with_extension_if_missing(&install_name, ".pnt");
                let destination = base_destination.join(file_name);
                if destination.exists() {
                    Err(format!(
                        "Destination already exists: {}.",
                        destination.display()
                    ))
                } else if let Err(err) = fs::copy(pnt_path, &destination) {
                    Err(format!(
                        "Failed to install .pnt file to {}: {}",
                        destination.display(),
                        err
                    ))
                } else {
                    Ok((
                        StatusKind::Success,
                        format!("Installed mod file to {}", destination.display()),
                    ))
                }
            }
        };

        match result {
            Ok((kind, msg)) => {
                self.set_status(kind, msg);
                pending.source.cleanup();
                self.refresh_mod_lists();
            }
            Err(msg) => {
                self.set_status(StatusKind::Error, msg);
                self.pending_install = Some(pending);
            }
        }
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
        let selected_target = {
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
            pending.install_target
        };

        self.last_install_target = selected_target;

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

    fn draw_mod_list(
        ui: &mut egui::Ui,
        title: &str,
        mods: &[ModEntry],
        interactive: bool,
    ) -> Option<ModEntry> {
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
                                if interactive && ui.button("Uninstall").clicked() {
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

        let has_pending_uninstall = self.pending_uninstall.is_some();

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

            let mut uninstall_target = None;
            egui::ScrollArea::vertical().show(ui, |ui| {
                for &target in &ALL_INSTALL_TARGETS {
                    let mods = self
                        .mod_lists
                        .get(&target)
                        .map(|v| v.as_slice())
                        .unwrap_or(&[]);
                    if let Some(entry) =
                        Self::draw_mod_list(ui, target.label(), mods, !has_pending_uninstall)
                    {
                        uninstall_target = Some(entry);
                    }
                }
            });

            if !has_pending_uninstall {
                self.pending_uninstall = uninstall_target;
            }
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
