// Exercise build-script logic during ordinary `cargo test`, as well as through
// the standalone `rustc --test build_support/mod.rs` maintainer command.
#[allow(dead_code)]
#[path = "../build_support/mod.rs"]
mod build_support;

use build_support::{Distribution, checksum, prepare, run, stage_native};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
const ORIGINAL_FILE: &[u8] = b"fixture contents: intact\n";

/// A tiny authenticated archive exercises real extraction and locking without
/// downloading or executing Python. The production manifest is unchanged.
struct CachedArchive {
    directory: PathBuf,
    cache: PathBuf,
    distribution: Distribution,
}

impl CachedArchive {
    fn new() -> Self {
        let target = if cfg!(windows) {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_os = "macos") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-unknown-linux-gnu"
        };
        Self::for_target(target)
    }

    fn for_target(target: &str) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "pylink offline fixture {}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let cache = directory.join("cache");
        let source = directory.join("source");
        let python = source.join("python");
        let mut distribution = Distribution::select("3.14", target).unwrap();
        for relative in distribution.required_files() {
            let file = python.join(relative);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, ORIGINAL_FILE).unwrap();
        }
        fs::write(
            distribution.include_dir(&python).join("patchlevel.h"),
            format!("#define PY_VERSION \"{}\"\n", distribution.version),
        )
        .unwrap();
        let archive = directory.join("fixture.tar.gz");
        run(Command::new(if cfg!(windows) { "tar.exe" } else { "tar" })
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("python"))
        .unwrap();
        // Distribution uses static strings because the production catalog is
        // compiled into the crate. A few fixture digests live until test exit.
        distribution.sha256 = Box::leak(checksum(&archive).unwrap().into_boxed_str());
        let entry = cache.join(distribution.cache_key());
        fs::create_dir_all(&entry).unwrap();
        fs::copy(&archive, entry.join(distribution.archive_name())).unwrap();
        Self {
            directory,
            cache,
            distribution,
        }
    }

    fn prepare(&self) -> PathBuf {
        prepare(&self.distribution, &self.cache, true).unwrap()
    }

    fn entry(&self) -> PathBuf {
        self.cache.join(self.distribution.cache_key())
    }
}

impl Drop for CachedArchive {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn assert_original(path: impl AsRef<Path>) {
    assert_eq!(fs::read(path).unwrap(), ORIGINAL_FILE);
}

#[test]
fn valid_archive_extracts_offline_and_valid_runtime_is_reused() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    fixture.distribution.validate_home(&home).unwrap();
    let sentinel = home.join("cache-was-reused");
    fs::write(&sentinel, "preserve this on a cache hit").unwrap();
    let config = fixture.entry().join("pyo3-config.txt");
    let old_time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    fs::File::options()
        .write(true)
        .open(&config)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old_time))
        .unwrap();
    let modified = config.metadata().unwrap().modified().unwrap();
    assert_eq!(fixture.prepare(), home);
    assert!(
        sentinel.exists(),
        "a valid cache hit unnecessarily re-extracted"
    );
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        fixture.distribution.pyo3_config()
    );
    assert_eq!(
        config.metadata().unwrap().modified().unwrap(),
        modified,
        "rewriting an unchanged config races concurrent PyO3 readers"
    );
}

#[test]
fn missing_required_file_is_repaired_offline() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    let relative = fixture
        .distribution
        .required_files()
        .into_iter()
        .find(|path| path.ends_with(Path::new("encodings/__init__.py")))
        .unwrap();
    fs::remove_file(home.join(&relative)).unwrap();
    assert_eq!(fixture.prepare(), home);
    assert_original(home.join(relative));
}

#[test]
fn missing_linker_library_and_windows_runtime_dll_are_repaired() {
    for (target, relative) in [
        ("x86_64-unknown-linux-gnu", "lib/libpython3.14.so"),
        ("x86_64-pc-windows-msvc", "vcruntime140_1.dll"),
    ] {
        let fixture = CachedArchive::for_target(target);
        let home = fixture.prepare();
        fs::remove_file(home.join(relative)).unwrap();
        fixture.prepare();
        assert_original(home.join(relative));
    }
}

#[test]
fn corrupt_library_is_repaired_from_valid_cached_archive() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    let library = home.join(&fixture.distribution.required_files()[0]);
    // Preserve length: existence/size-only validation cannot detect this damage.
    fs::write(&library, vec![b'!'; ORIGINAL_FILE.len()]).unwrap();
    assert_eq!(fixture.prepare(), home);
    assert_original(library);
}

#[test]
fn incorrect_version_header_and_missing_completion_marker_are_repaired() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    let header = fixture.distribution.include_dir(&home).join("patchlevel.h");
    fs::write(&header, "#define PY_VERSION \"3.0.0\"\n").unwrap();
    fixture.prepare();
    fixture.distribution.validate_home(&home).unwrap();

    let marker = home.parent().unwrap().join(".complete");
    fs::remove_file(&marker).unwrap();
    // Failed extraction remnants from an interrupted build must be discarded.
    let unpack = fixture.entry().join("unpack");
    fs::create_dir_all(&unpack).unwrap();
    fs::write(unpack.join("partial-file"), "unfinished").unwrap();
    fixture.prepare();
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        fixture.distribution.sha256
    );
    assert!(!unpack.exists());
    assert!(!home.parent().unwrap().join("partial-file").exists());
}

#[test]
fn incorrect_and_missing_bundled_version_marker_are_repaired() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    let marker = home.join(".pylink-version");
    fs::write(&marker, "3.0.0").unwrap();
    fixture.prepare();
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        fixture.distribution.version
    );
    fs::remove_file(&marker).unwrap();
    fixture.prepare();
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        fixture.distribution.version
    );
}

#[test]
fn corrupt_archive_is_rejected_offline_even_with_complete_runtime() {
    let fixture = CachedArchive::new();
    let home = fixture.prepare();
    let archive = fixture.entry().join(fixture.distribution.archive_name());
    fs::write(archive, "not the authenticated archive").unwrap();
    let error = prepare(&fixture.distribution, &fixture.cache, true).unwrap_err();
    assert!(error.contains("offline"), "unexpected failure: {error}");
    assert!(home.exists());
    assert!(!fixture.entry().join("download.part").exists());
}

#[test]
fn concurrent_builds_publish_one_complete_runtime() {
    let fixture = CachedArchive::new();
    let start = Arc::new(Barrier::new(4));
    let workers: Vec<_> = (0..4)
        .map(|_| {
            let start = Arc::clone(&start);
            let distribution = fixture.distribution.clone();
            let cache = fixture.cache.clone();
            std::thread::spawn(move || {
                start.wait();
                prepare(&distribution, &cache, true).unwrap()
            })
        })
        .collect();
    let homes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(homes.iter().all(|home| home == &homes[0]));
    fixture.distribution.validate_home(&homes[0]).unwrap();
    assert_original(homes[0].join(&fixture.distribution.required_files()[0]));
    assert!(!fixture.entry().join("unpack").exists());
    assert!(!fixture.entry().join("download.part").exists());
}

#[test]
fn windows_native_staging_copies_runtime_dlls_and_pyo3_import_alias() {
    // Staging Windows files requires no Windows executables, so every host can
    // check the import-library convention used by PyO3's suppressed build script.
    let fixture = CachedArchive::for_target("x86_64-pc-windows-msvc");
    let home = fixture.prepare();
    fs::write(home.join("vcruntime140.dll"), ORIGINAL_FILE).unwrap();
    fs::write(home.join("python3.dll"), ORIGINAL_FILE).unwrap();
    fs::write(home.join("unrelated.txt"), ORIGINAL_FILE).unwrap();
    let output = fixture.directory.join("native");
    fs::create_dir(&output).unwrap();
    fs::write(output.join("stale.lib"), ORIGINAL_FILE).unwrap();
    stage_native(&fixture.distribution, &home, &output).unwrap();
    for name in [
        "python314.lib",
        "pythonXY.lib",
        "python314.dll",
        "python3.dll",
        "vcruntime140.dll",
    ] {
        assert_original(output.join(name));
    }
    assert!(!output.join("unrelated.txt").exists());
    assert!(!output.join("stale.lib").exists());
}

#[cfg(unix)]
#[test]
fn linux_native_staging_materializes_the_linker_symlink() {
    let fixture = CachedArchive::for_target("x86_64-unknown-linux-gnu");
    let home = fixture.prepare();
    let linker_path = home.join("lib/libpython3.14.so");
    if linker_path.exists() {
        fs::remove_file(&linker_path).unwrap();
    }
    std::os::unix::fs::symlink("libpython3.14.so.1.0", linker_path).unwrap();
    let output = fixture.directory.join("native");
    stage_native(&fixture.distribution, &home, &output).unwrap();
    assert_original(output.join("libpython3.14.so.1.0"));
    let link = output.join("libpython3.14.so");
    assert_original(&link);
    assert!(!link.symlink_metadata().unwrap().is_symlink());
}
