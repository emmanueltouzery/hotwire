use gtk::prelude::*;
use serde_derive::{Deserialize, Serialize};
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Sync + Send>>;

#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Config {
    pub prefer_dark_theme: bool,
    pub custom_tcpdump_buffer_size_kib: Option<usize>,
    pub tcpdump_use_pkexec_if_possible: bool,
}

impl Config {
    pub fn default_config() -> Config {
        Config {
            prefer_dark_theme: false,
            custom_tcpdump_buffer_size_kib: Some(8192),
            tcpdump_use_pkexec_if_possible: true,
        }
    }

    pub fn config_file_path() -> Result<PathBuf> {
        let config_folder = config_folder()?;
        Ok(config_folder.join("config.toml"))
    }

    fn read_config_file() -> Result<Config> {
        let config_file = Self::config_file_path()?;
        if !config_file.is_file() {
            return Ok(Self::default_config());
        }
        let mut contents = String::new();
        File::open(config_file)?.read_to_string(&mut contents)?;
        let r = toml::from_str(&contents)?;
        Ok(r)
    }

    pub fn read_config() -> Config {
        Self::read_config_file().unwrap_or_else(|e| {
            let dialog = gtk::MessageDialog::new(
                None::<&gtk::Window>,
                gtk::DialogFlags::all(),
                gtk::MessageType::Error,
                gtk::ButtonsType::Close,
                "Error loading the configuration",
            );
            dialog.set_property_secondary_text(Some(&format!(
                "{}: {:}",
                Self::config_file_path()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| "".to_string()),
                e
            )));
            let _r = dialog.run();
            dialog.close();
            Self::default_config()
        })
    }

    fn save_config_file(&self) -> Result<()> {
        let mut file = File::create(Self::config_file_path()?)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn save_config(&self, parent_win: &gtk::Window) {
        self.save_config_file().unwrap_or_else(|e| {
            let dialog = gtk::MessageDialog::new(
                Some(parent_win),
                gtk::DialogFlags::all(),
                gtk::MessageType::Error,
                gtk::ButtonsType::Close,
                "Error saving the configuration",
            );
            dialog.set_property_secondary_text(Some(&format!("{}", e)));
            let _r = dialog.run();
            dialog.close();
        });
    }
}

pub fn data_folder() -> Result<PathBuf> {
    let data_dir = dirs::data_dir().ok_or("Can't find your data folder?")?;
    let data_folder = data_dir.join("hotwire");
    if !data_folder.is_dir() {
        std::fs::create_dir(&data_folder)?;
    }
    Ok(data_folder)
}

pub fn config_folder() -> Result<PathBuf> {
    let data_dir = dirs::config_dir().ok_or("Can't find your config folder?")?;
    let config_folder = data_dir.join("hotwire");
    if !config_folder.is_dir() {
        std::fs::create_dir(&config_folder)?;
    }
    Ok(config_folder)
}

#[derive(PartialEq, Eq)]
pub enum RemoveMode {
    OldFilesOnly,
    OldFilesAndMyFiles,
}

// can't blindly delete all files, because the app might be running
// multiple times concurrently...
pub fn remove_obsolete_tcpdump_files(remove_mode: RemoveMode) -> Result<()> {
    let fifo_path = data_folder()?;
    let paths = fs::read_dir(fifo_path)?;
    let is_old = |p: &std::path::Path| {
        fs::metadata(p)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|m| m.elapsed().ok())
            .filter(|d| d >= &std::time::Duration::from_secs(24 * 3600 * 5))
            .is_some()
    };
    let is_my_file = |f: &str| {
        f.replace(".pcap", "")
            .ends_with(&format!("-{}", std::process::id()))
    };
    for direntry in paths {
        let path = direntry?.path();
        let should_remove = path
            .file_name()
            .and_then(|f| f.to_str())
            .filter(|f| f.starts_with("hotwire-record-") || f.starts_with("hotwire-save-"))
            .filter(|f| {
                is_old(&path) || (remove_mode == RemoveMode::OldFilesAndMyFiles && is_my_file(&f))
            })
            .is_some();
        if should_remove {
            if let Err(e) = fs::remove_file(&path) {
                eprintln!("Failed to remove obsolete tcpdump fifo: {:?}, {}", path, e);
            }
        }
    }
    Ok(())
}

pub fn get_tcpdump_fifo_path() -> PathBuf {
    let mut fifo_path = data_folder().unwrap();
    fifo_path.push(format!("hotwire-record-{}", std::process::id()));
    fifo_path
}

pub fn get_tshark_pcap_output_path() -> PathBuf {
    let mut pcap_path = data_folder().unwrap();
    pcap_path.push(format!("hotwire-save-{}.pcap", std::process::id()));
    pcap_path
}
