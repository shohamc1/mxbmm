use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use notify::RecommendedWatcher;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum InstallTarget {
    Tracks,
    BikesMotocross,
    BikesSupercross,
    BikesPaints,
    Tyres,
    RiderModels,
    RiderPaints,
    RiderGloves,
    RiderHelmets,
    RiderHelmetPaints,
    RiderBoots,
    RiderBootPaints,
    RiderProtections,
}

impl InstallTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tracks => "Tracks",
            Self::BikesMotocross => "Bikes Motocross",
            Self::BikesSupercross => "Bikes Supercross",
            Self::BikesPaints => "Bike Paints",
            Self::Tyres => "Tyres/Wheels",
            Self::RiderModels => "Rider Models",
            Self::RiderPaints => "Rider Paints",
            Self::RiderGloves => "Rider Gloves",
            Self::RiderHelmets => "Helmet Models",
            Self::RiderHelmetPaints => "Helmet Paints",
            Self::RiderBoots => "Boot Models",
            Self::RiderBootPaints => "Boot Paints",
            Self::RiderProtections => "Protections",
        }
    }

    pub fn relative_path(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::BikesMotocross => "bikes/motocross",
            Self::BikesSupercross => "bikes/supercross",
            Self::BikesPaints => "bikes/paints",
            Self::Tyres => "tyres",
            Self::RiderModels => "rider/riders",
            Self::RiderPaints => "rider/riders/paints",
            Self::RiderGloves => "rider/riders/gloves",
            Self::RiderHelmets => "rider/helmets",
            Self::RiderHelmetPaints => "rider/helmets/paints",
            Self::RiderBoots => "rider/boots",
            Self::RiderBootPaints => "rider/boots/paints",
            Self::RiderProtections => "rider/protections",
        }
    }

    pub fn excluded_subdirs(self) -> &'static [&'static str] {
        match self {
            Self::RiderModels => &["paints", "gloves"],
            Self::RiderHelmets | Self::RiderBoots => &["paints"],
            _ => &[],
        }
    }
}

pub const ALL_INSTALL_TARGETS: [InstallTarget; 13] = [
    InstallTarget::Tracks,
    InstallTarget::BikesMotocross,
    InstallTarget::BikesSupercross,
    InstallTarget::BikesPaints,
    InstallTarget::Tyres,
    InstallTarget::RiderModels,
    InstallTarget::RiderPaints,
    InstallTarget::RiderGloves,
    InstallTarget::RiderHelmets,
    InstallTarget::RiderHelmetPaints,
    InstallTarget::RiderBoots,
    InstallTarget::RiderBootPaints,
    InstallTarget::RiderProtections,
];

#[derive(Clone)]
pub struct ModEntry {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy)]
pub enum StatusKind {
    Info,
    Success,
    Error,
}

pub struct StatusMessage {
    pub kind: StatusKind,
    pub text: String,
}

pub enum PendingSource {
    Zip {
        archive_path: PathBuf,
        temp_extract_dir: PathBuf,
    },
    Pkz {
        pkz_path: PathBuf,
    },
    Pnt {
        pnt_path: PathBuf,
    },
}

impl PendingSource {
    pub fn input_path(&self) -> &Path {
        match self {
            Self::Zip { archive_path, .. } => archive_path,
            Self::Pkz { pkz_path } => pkz_path,
            Self::Pnt { pnt_path } => pnt_path,
        }
    }

    pub fn cleanup(&self) {
        if let Self::Zip {
            temp_extract_dir, ..
        } = self
        {
            let _ = fs::remove_dir_all(temp_extract_dir);
        }
    }
}

pub struct PendingInstall {
    pub source: PendingSource,
    pub install_target: InstallTarget,
    pub custom_name: String,
    pub notes: String,
    pub version: String,
}

pub struct FsWatcherState {
    pub root: PathBuf,
    pub _watcher: RecommendedWatcher,
    pub rx: Receiver<notify::Result<notify::Event>>,
}
