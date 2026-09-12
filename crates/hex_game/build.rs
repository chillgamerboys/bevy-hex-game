//! Build the optional native recorder without adding an unsafe Rust boundary.

use std::{env, error::Error, path::PathBuf, process::Command};

#[expect(
    clippy::print_stdout,
    reason = "Cargo build-script directives must be written to stdout."
)]
fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=native/recorder/Recorder.swift");
    println!("cargo:rerun-if-changed=native/recorder/Info.plist");
    println!("cargo:rerun-if-env-changed=HEX_GAME_BUILD_ID");
    let current_reference = Command::new("git")
        .args(["symbolic-ref", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    for reference in std::iter::once("HEAD").chain(current_reference.as_deref()) {
        if let Ok(path) = Command::new("git")
            .args(["rev-parse", "--git-path", reference])
            .output()
        {
            if path.status.success() {
                println!(
                    "cargo:rerun-if-changed={}",
                    String::from_utf8_lossy(&path.stdout).trim()
                );
            }
        }
    }
    if env::var("CARGO_CFG_TARGET_OS")?.as_str() != "macos"
        || env::var_os("CARGO_FEATURE_ARENA_PROTOTYPE").is_none()
    {
        return Ok(());
    }
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH")?.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => return Err(format!("unsupported recorder architecture: {other}").into()),
    };
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo OUT_DIR is missing")?)
        .join("hex-game-recorder");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-swift-version",
            "5",
            "-parse-as-library",
            "-target",
        ])
        .arg(format!("{architecture}-apple-macos12.3"))
        .args(["native/recorder/Recorder.swift", "-o"])
        .arg(&output)
        .args([
            "-Xlinker",
            "-sectcreate",
            "-Xlinker",
            "__TEXT",
            "-Xlinker",
            "__info_plist",
            "-Xlinker",
            "native/recorder/Info.plist",
        ])
        .status()?;
    if !status.success() {
        return Err("native recorder compilation failed; install an Apple SDK with macOS 15 ScreenCaptureKit APIs".into());
    }
    if !Command::new("/usr/bin/codesign")
        .args([
            "--force",
            "--sign",
            "-",
            "--identifier",
            "com.chillgamerboys.hex-game.recorder",
        ])
        .arg(&output)
        .status()?
        .success()
    {
        return Err("native recorder ad-hoc signing failed".into());
    }
    println!("cargo:rustc-env=HEX_RECORDER_BINARY={}", output.display());
    let build = env::var("HEX_GAME_BUILD_ID")
        .ok()
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=HEX_RECORDER_BUILD_ID={build}");
    Ok(())
}
