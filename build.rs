use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=native/macos.m");
    println!("cargo:rerun-if-changed=native/window_chrome.m");
    println!("cargo:rerun-if-changed=native/bridge.h");
    let target_is_macos = env::var("CARGO_CFG_TARGET_OS").is_ok_and(|value| value == "macos");
    if target_is_macos && env::var_os("CARGO_FEATURE_PORTABLE").is_some() {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
            "aarch64" => "arm64",
            "x86_64" => "x86_64",
            other => panic!("unsupported macOS architecture: {other}"),
        };
        let object = out.join("window_chrome.o");
        assert!(
            Command::new("xcrun")
                .args([
                    "clang",
                    "-arch",
                    arch,
                    "-mmacosx-version-min=12.0",
                    "-fobjc-arc",
                    "-fmodules",
                    "-O2",
                    "-Wall",
                    "-Wextra",
                    "-Werror",
                    "-c",
                    "native/window_chrome.m",
                    "-o"
                ])
                .arg(&object)
                .status()
                .expect("Xcode Command Line Tools required")
                .success()
        );
        assert!(
            Command::new("ar")
                .arg("crs")
                .arg(out.join("libbestgui_chrome.a"))
                .arg(object)
                .status()
                .unwrap()
                .success()
        );
        println!("cargo:rustc-link-search=native={}", out.display());
        println!("cargo:rustc-link-lib=static=bestgui_chrome");
        println!("cargo:rustc-link-lib=framework=AppKit");
    }
    if env::var_os("CARGO_FEATURE_NATIVE").is_none() {
        return;
    }
    assert_eq!(
        env::var("CARGO_CFG_TARGET_OS").unwrap(),
        "macos",
        "The native demo currently supports macOS only"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture: {other}"),
    };
    let status = Command::new("xcrun")
        .args([
            "clang",
            "-arch",
            arch,
            "-mmacosx-version-min=12.0",
            "-fobjc-arc",
            "-fmodules",
            "-O2",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-c",
            "native/macos.m",
            "-o",
        ])
        .arg(out.join("macos.o"))
        .status()
        .expect("Xcode Command Line Tools required");
    assert!(status.success(), "native adapter compilation failed");
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libbestgui_native.a"))
            .arg(out.join("macos.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=bestgui_native");
    for framework in ["AppKit", "Metal", "QuartzCore"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
