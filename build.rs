use flate2::read::GzDecoder;
use std::fs::*;
use std::path::Path;
use std::process::Command;

const FONTAWESOME_VERSION: &str = "5.13.0";

fn main() {
    println!("cargo:rerun-if-changed=src/icons.gresource");
    let target_foldername = format!("fontawesome-{}", FONTAWESOME_VERSION);
    if !Path::new(&target_foldername).exists() {
        fetch_fontawesome_icons(&target_foldername);
    }

    let status = Command::new("glib-compile-resources")
        .arg("src/icons.gresource")
        .arg("--target=src/icons.bin")
        .spawn()
        .expect("Failed running glib-compile-resources")
        .wait()
        .unwrap();
    assert!(status.success());
}

fn fetch_fontawesome_icons(target_foldername: &str) {
    let fontawesome_url = format!(
        "https://registry.npmjs.org/@fortawesome/fontawesome-free/-/fontawesome-free-{}.tgz",
        FONTAWESOME_VERSION
    );
    let mut resp = reqwest::blocking::get(&fontawesome_url).expect("request failed");
    let mut out = File::create("fontawesome.tgz").expect("failed to create file");
    std::io::copy(&mut resp, &mut out).expect("failed to copy content");
    let mut archive = tar::Archive::new(GzDecoder::new(
        File::open("fontawesome.tgz").expect("open archive"),
    ));
    archive.unpack(".").expect("Failed extracting");
    rename("package", target_foldername).expect("folder rename");
    remove_file("fontawesome.tgz").expect("remove tgz");
}
