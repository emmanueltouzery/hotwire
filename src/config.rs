use std::error::Error;
use std::fs;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Box<dyn Error + Sync + Send>>;

pub fn config_folder() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().expect("Can't find your home folder?");
    let config_folder = home_dir.join(".hotwire");
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
    let fifo_path = config_folder()?;
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
        if path
            .file_name()
            .and_then(|f| f.to_str())
            .filter(|f| f.starts_with("hotwire-record-") || f.starts_with("hotwire-save-"))
            .filter(|f| {
                is_old(&path) || (remove_mode == RemoveMode::OldFilesAndMyFiles && is_my_file(&f))
            })
            .is_some()
        {
            if let Err(e) = fs::remove_file(&path) {
                eprintln!("Failed to remove obsolete tcpdump fifo: {:?}, {}", path, e);
            }
        }
    }
    Ok(())
}

pub fn get_tcpdump_fifo_path() -> PathBuf {
    let mut fifo_path = config_folder().unwrap();
    fifo_path.push(format!("hotwire-record-{}", std::process::id()));
    fifo_path
}

pub fn get_tshark_pcap_output_path() -> PathBuf {
    let mut pcap_path = config_folder().unwrap();
    pcap_path.push(format!("hotwire-save-{}.pcap", std::process::id()));
    pcap_path
}
