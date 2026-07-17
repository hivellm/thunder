//! Captures the compiling rustc's version string so the artifact writer can
//! put it in every environment header (BEN-011) via
//! `env!("THUNDER_BENCH_RUSTC")`.

use std::process::Command;

fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let version = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=THUNDER_BENCH_RUSTC={version}");
    println!("cargo:rerun-if-changed=build.rs");
}
