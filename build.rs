mod build_support;

use std::{env, path::PathBuf};

fn main() {
    if let Err(error) = build() {
        panic!("pylink: {error}");
    }
}

fn build() -> build_support::Result<()> {
    for file in [
        "build.rs",
        "build_support/mod.rs",
        "build_support/distributions.tsv",
    ] {
        println!("cargo:rerun-if-changed={file}");
    }
    for name in [
        "PYLINK_PYTHON_VERSION",
        "PYLINK_VERSION_FILE",
        "PYLINK_CACHE_DIR",
        "PYLINK_OFFLINE",
        "PYLINK_LINK_MODE",
        "CARGO_NET_OFFLINE",
        "DOCS_RS",
        "HOME",
        "LOCALAPPDATA",
        "XDG_CACHE_HOME",
    ] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    if env::var("PYLINK_LINK_MODE").unwrap_or_else(|_| "dynamic".into()) != "dynamic" {
        return Err("only PYLINK_LINK_MODE=dynamic is supported; Astral's install_only_stripped distributions do not contain a static libpython".into());
    }
    let target = env::var("TARGET").map_err(|e| e.to_string())?;
    let version = build_support::requested_version()?;
    let distribution = build_support::Distribution::select(&version, &target)?;
    if env::var_os("DOCS_RS").is_some() {
        // docs.rs has no network access. Rustdoc needs the version constants,
        // but no interpreter; these empty paths are documentation-only placeholders.
        println!(
            "cargo:rustc-env=PYLINK_PYTHON_VERSION={}",
            distribution.version
        );
        println!(
            "cargo:rustc-env=PYLINK_PYTHON_RELEASE={}",
            distribution.release
        );
        println!("cargo:rustc-env=PYLINK_PYTHON_HOME=");
        println!("cargo:rustc-env=PYLINK_PYO3_CONFIG_FILE=");
        return Ok(());
    }
    let cache = build_support::cache_dir()?;
    let offline =
        build_support::flag("PYLINK_OFFLINE")? || build_support::flag("CARGO_NET_OFFLINE")?;
    let home = build_support::prepare(&distribution, &cache, offline)?;
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is unset")?);
    let native = out.join("native");
    build_support::stage_native(&distribution, &home, &native)?;

    println!("cargo:rustc-link-search=native={}", native.display());
    println!("cargo:rustc-link-lib=dylib={}", distribution.lib_name());
    println!("cargo:rustc-env=PYLINK_PYTHON_HOME={}", home.display());
    println!(
        "cargo:rustc-env=PYLINK_PYTHON_VERSION={}",
        distribution.version
    );
    println!(
        "cargo:rustc-env=PYLINK_PYTHON_RELEASE={}",
        distribution.release
    );
    let config = home
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("pyo3-config.txt");
    println!(
        "cargo:rustc-env=PYLINK_PYO3_CONFIG_FILE={}",
        config.display()
    );
    println!("cargo:python_home={}", home.display());
    println!("cargo:pyo3_config_file={}", config.display());
    println!("cargo:python_version={}", distribution.version);
    // Detect an evicted/damaged cache on the next Cargo invocation.
    println!(
        "cargo:rerun-if-changed={}",
        home.parent().unwrap().join(".complete").display()
    );
    for file in [
        home.join(".pylink-version"),
        home.parent().unwrap().join(".critical-hashes"),
        home.parent()
            .unwrap()
            .parent()
            .unwrap()
            .join(distribution.archive_name()),
    ] {
        println!("cargo:rerun-if-changed={}", file.display());
    }
    for file in distribution.required_files() {
        println!("cargo:rerun-if-changed={}", home.join(file).display());
    }
    Ok(())
}
