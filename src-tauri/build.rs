// Builds the tauri context, plus the razer_dyn LADSPA plugin (dsp/razer_dyn.c) that enhance.rs ships inside the binary.
use std::{env, path::PathBuf, process::Command};

fn main() {
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../dsp/razer_dyn.c");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("razer_dyn.so");
    println!("cargo:rerun-if-changed={}", src.display());
    let ok = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()))
        .args(["-O2", "-Wall", "-shared", "-fPIC", "-o"])
        .arg(&out)
        .arg(&src)
        .arg("-lm")
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "failed to compile {} (needs a C compiler and ladspa.h from the 'ladspa' package)", src.display());
    tauri_build::build()
}
