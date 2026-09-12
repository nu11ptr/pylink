//! Build-time download/cache code, also testable directly with `rustc --test`.
use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub type Result<T> = std::result::Result<T, String>;
pub const DEFAULT_VERSION: &str = "3.14";
const CATALOG: &str = include_str!("distributions.tsv");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Distribution {
    pub version: &'static str,
    pub release: &'static str,
    pub target: &'static str,
    pub sha256: &'static str,
}

impl Distribution {
    pub fn select(version: &str, target: &str) -> Result<Self> {
        let parts: Vec<_> = version.split('.').collect();
        if !(parts.len() == 2 || parts.len() == 3)
            || parts.iter().any(|p| {
                p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) || p.parse::<u32>().is_err()
            })
        {
            return Err(format!(
                "invalid Python version {version:?}; use e.g. 3.14 or 3.14.7"
            ));
        }
        if (
            parts[0].parse::<u32>().unwrap(),
            parts[1].parse::<u32>().unwrap(),
        ) < (3, 14)
        {
            return Err(format!(
                "Python {version} is unsupported: pylink requires Python 3.14 or newer for the opaque PyInitConfig API; set PYLINK_PYTHON_VERSION=3.14 or update your PYLINK_VERSION_FILE"
            ));
        }
        let mut available = Vec::new();
        for line in CATALOG
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 4, "invalid bundled distribution catalog");
            if fields[2] != target {
                continue;
            }
            available.push(fields[0]);
            if fields[0] == version
                || (parts.len() == 2 && fields[0].starts_with(&format!("{version}.")))
            {
                return Ok(Self {
                    version: fields[0],
                    release: fields[1],
                    target: fields[2],
                    sha256: fields[3],
                });
            }
        }
        if available.is_empty() {
            Err(format!(
                "unsupported Rust target {target:?}; supported: aarch64/x86_64-apple-darwin, aarch64/x86_64-unknown-linux-gnu, aarch64/x86_64-pc-windows-msvc (dynamic, GIL-enabled Python only)"
            ))
        } else {
            Err(format!(
                "Python {version} is not in this crate's pinned catalog; available patch versions for {target}: {}. Choose a listed version or update pylink for a newer catalog",
                available.join(", ")
            ))
        }
    }
    pub fn minor(&self) -> &str {
        self.version.rsplit_once('.').unwrap().0
    }
    pub fn windows(&self) -> bool {
        self.target.contains("windows")
    }
    pub fn macos(&self) -> bool {
        self.target.contains("apple-darwin")
    }
    pub fn lib_name(&self) -> String {
        format!(
            "python{}",
            if self.windows() {
                self.minor().replace('.', "")
            } else {
                self.minor().into()
            }
        )
    }
    pub fn archive_name(&self) -> String {
        format!(
            "cpython-{}+{}-{}-install_only_stripped.tar.gz",
            self.version, self.release, self.target
        )
    }
    pub fn url(&self) -> String {
        format!(
            "https://github.com/astral-sh/python-build-standalone/releases/download/{}/{}",
            self.release,
            self.archive_name()
        )
    }
    pub fn cache_key(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.version, self.release, self.target, self.sha256
        )
    }
    pub fn include_dir(&self, home: &Path) -> PathBuf {
        if self.windows() {
            home.join("include")
        } else {
            home.join("include").join(format!("python{}", self.minor()))
        }
    }
    pub fn required_files(&self) -> Vec<PathBuf> {
        let mut files = if self.windows() {
            vec![
                PathBuf::from(format!("{}.dll", self.lib_name())),
                PathBuf::from(format!("libs/{}.lib", self.lib_name())),
                PathBuf::from("python3.dll"),
                PathBuf::from("vcruntime140.dll"),
                PathBuf::from("vcruntime140_1.dll"),
                PathBuf::from("Lib/encodings/__init__.py"),
            ]
        } else {
            let suffix = if self.macos() { "dylib" } else { "so.1.0" };
            vec![
                PathBuf::from(format!("lib/lib{}.{}", self.lib_name(), suffix)),
                PathBuf::from(format!("lib/python{}/encodings/__init__.py", self.minor())),
            ]
        };
        if !self.windows() && !self.macos() {
            files.push(PathBuf::from(format!("lib/lib{}.so", self.lib_name())));
        }
        // No C headers are compiled. Keep the version header solely to verify
        // that the extracted distribution matches the pinned patch version.
        files.push(self.include_dir(Path::new("")).join("patchlevel.h"));
        files
    }
    pub fn validate_home(&self, home: &Path) -> Result<()> {
        for relative in self.required_files() {
            if !home.join(&relative).is_file() {
                return Err(format!(
                    "incomplete Python distribution: missing {}",
                    home.join(relative).display()
                ));
            }
        }
        let header =
            fs::read_to_string(self.include_dir(home).join("patchlevel.h")).map_err(io_error)?;
        if !header.lines().any(|line| {
            line.split_whitespace().collect::<Vec<_>>()
                == ["#define", "PY_VERSION", &format!("\"{}\"", self.version)]
        }) {
            return Err(format!(
                "Python version header does not match pinned version {}",
                self.version
            ));
        }
        Ok(())
    }
    pub fn pyo3_config(&self) -> String {
        format!(
            "implementation=CPython\nversion={}\nshared=true\nabi3=false\nlib_name={}\nsuppress_build_script_link_lines=true\n",
            self.minor(),
            self.lib_name()
        )
    }
}

fn io_error(error: std::io::Error) -> String {
    error.to_string()
}

pub fn absolute_path(path: PathBuf, setting: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(format!(
            "{setting} must be an absolute path, got {} (use {{ value = \"...\", relative = true }} in .cargo/config.toml)",
            path.display()
        ));
    }
    if path.to_str().is_none_or(|p| p.contains(['\n', '\r'])) {
        return Err(format!(
            "{setting} must be valid Unicode without newlines for Cargo build directives"
        ));
    }
    Ok(path)
}

pub fn requested_version() -> Result<String> {
    let selected = env::var("PYLINK_PYTHON_VERSION").map_err(|e| e.to_string());
    // Track the file even while overridden, so switching back sees its current content.
    let file = env::var_os("PYLINK_VERSION_FILE").map(PathBuf::from);
    if let Some(ref file) = file {
        let file = absolute_path(file.clone(), "PYLINK_VERSION_FILE")?;
        println!("cargo:rerun-if-changed={}", file.display());
    }
    if env::var_os("PYLINK_PYTHON_VERSION").is_some() {
        return selected;
    }
    match file {
        Some(file) => fs::read_to_string(&file)
            .map(|s| s.trim().to_owned())
            .map_err(|e| format!("cannot read version file {}: {e}", file.display())),
        None => Ok(DEFAULT_VERSION.to_owned()),
    }
}

pub fn cache_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("PYLINK_CACHE_DIR") {
        return absolute_path(PathBuf::from(path), "PYLINK_CACHE_DIR");
    }
    let base = if cfg!(windows) {
        PathBuf::from(
            env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unset; set PYLINK_CACHE_DIR")?,
        )
        .join("pylink")
        .join("Cache")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(env::var_os("HOME").ok_or("HOME is unset; set PYLINK_CACHE_DIR")?)
            .join("Library/Caches/pylink")
    } else if let Some(path) = env::var_os("XDG_CACHE_HOME").filter(|p| Path::new(p).is_absolute())
    {
        PathBuf::from(path).join("pylink")
    } else {
        PathBuf::from(env::var_os("HOME").ok_or("HOME is unset; set PYLINK_CACHE_DIR")?)
            .join(".cache/pylink")
    };
    absolute_path(base, "platform cache directory")
}

pub fn flag(name: &str) -> Result<bool> {
    match env::var(name).as_deref() {
        Ok("1" | "true") => Ok(true),
        Ok("0" | "false" | "") | Err(env::VarError::NotPresent) => Ok(false),
        _ => Err(format!("{name} must be true/false or 1/0")),
    }
}

pub fn run(command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .map_err(|e| format!("could not run {command:?}: {e}; see README build prerequisites"))?;
    if !output.status.success() {
        return Err(format!(
            "{command:?} failed ({}):\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("command produced non-UTF-8 output: {e}"))
}

pub fn checksum(path: &Path) -> Result<String> {
    let output = if cfg!(windows) {
        // Use .NET directly: a PSModulePath inherited from PowerShell 7 can
        // prevent Windows PowerShell from loading the Get-FileHash cmdlet.
        run(Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                r#"
$ErrorActionPreference = 'Stop'
$hasher = [System.Security.Cryptography.SHA256]::Create()
try {
    $stream = [System.IO.File]::OpenRead($env:PYLINK_HASH_FILE)
    try {
        [System.BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-', '')
    } finally {
        $stream.Dispose()
    }
} finally {
    $hasher.Dispose()
}
"#,
            ])
            .env("PYLINK_HASH_FILE", path))?
    } else if cfg!(target_os = "macos") {
        run(Command::new("shasum").args(["-a", "256"]).arg(path))?
    } else {
        run(Command::new("sha256sum").arg(path))?
    };
    let hash = output
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid SHA-256 output for {}", path.display()));
    }
    Ok(hash)
}

// OS locks are released even if a build is killed. Never unlink the lock file:
// another process may already hold an open descriptor for the same inode.
fn acquire_lock(path: &Path) -> Result<File> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(io_error)?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock)
                if start.elapsed() < Duration::from_secs(120) =>
            {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(format!("could not lock cache {}: {e}", path.display())),
        }
    }
}

pub fn prepare(distribution: &Distribution, cache: &Path, offline: bool) -> Result<PathBuf> {
    fs::create_dir_all(cache).map_err(io_error)?;
    let key = distribution.cache_key();
    let _lock = acquire_lock(&cache.join(format!("{key}.lock")))?;
    let entry = cache.join(key);
    fs::create_dir_all(&entry).map_err(io_error)?;
    let archive = entry.join(distribution.archive_name());
    let runtime = entry.join("runtime");
    let home = runtime.join("python");
    let mut valid_archive = archive.is_file() && checksum(&archive)? == distribution.sha256;
    if !valid_archive {
        if offline {
            return Err(format!(
                "offline: no valid cached archive for Python {} ({}) in {}; build once online or restore this cache",
                distribution.version,
                distribution.target,
                entry.display()
            ));
        }
        let partial = entry.join("download.part");
        let _ = fs::remove_file(&partial);
        eprintln!("pylink: downloading {}", distribution.url());
        run(
            Command::new(if cfg!(windows) { "curl.exe" } else { "curl" })
                .args([
                    "--fail",
                    "--location",
                    "--silent",
                    "--show-error",
                    "--retry",
                    "3",
                    "--connect-timeout",
                    "30",
                    "--max-time",
                    "600",
                    "--proto",
                    "=https",
                    "--proto-redir",
                    "=https",
                    "--output",
                ])
                .arg(&partial)
                .arg(distribution.url()),
        )?;
        let actual = checksum(&partial)?;
        if actual != distribution.sha256 {
            let _ = fs::remove_file(&partial);
            return Err(format!(
                "SHA-256 mismatch for {}: expected {}, got {actual}; refusing to extract",
                distribution.archive_name(),
                distribution.sha256
            ));
        }
        if archive.exists() {
            fs::remove_file(&archive).map_err(io_error)?;
        }
        fs::rename(&partial, &archive).map_err(io_error)?;
        valid_archive = false; // Re-extract whenever a missing/corrupt archive was repaired.
    }
    let marker = runtime.join(".complete");
    let critical_hashes = runtime.join(".critical-hashes");
    if !valid_archive
        || fs::read_to_string(&marker).ok().as_deref() != Some(distribution.sha256)
        || distribution.validate_home(&home).is_err()
        || fs::read_to_string(home.join(".pylink-version"))
            .ok()
            .as_deref()
            != Some(distribution.version)
        || !fs::read_to_string(&critical_hashes)
            .ok()
            .zip(critical_fingerprint(distribution, &home).ok())
            .is_some_and(|(expected, actual)| expected == actual)
    {
        let unpack = entry.join("unpack");
        if unpack.exists() {
            fs::remove_dir_all(&unpack).map_err(io_error)?;
        }
        fs::create_dir(&unpack).map_err(io_error)?;
        // Only authenticated, pinned upstream archives reach tar. Extract into a
        // private staging directory and publish only after validating the layout.
        run(Command::new(if cfg!(windows) { "tar.exe" } else { "tar" })
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&unpack))?;
        distribution.validate_home(&unpack.join("python"))?;
        fs::write(unpack.join("python/.pylink-version"), distribution.version).map_err(io_error)?;
        fs::write(unpack.join(".complete"), distribution.sha256).map_err(io_error)?;
        fs::write(
            unpack.join(".critical-hashes"),
            critical_fingerprint(distribution, &unpack.join("python"))?,
        )
        .map_err(io_error)?;
        if runtime.exists() {
            fs::remove_dir_all(&runtime).map_err(io_error)?;
        }
        fs::rename(unpack, &runtime).map_err(io_error)?;
    }
    let config = entry.join("pyo3-config.txt");
    let contents = distribution.pyo3_config();
    if fs::read_to_string(&config).ok().as_deref() != Some(&contents) {
        // Publish a complete config; PyO3 may read it from another Cargo process.
        let temporary = entry.join("pyo3-config.tmp");
        fs::write(&temporary, contents).map_err(io_error)?;
        fs::rename(temporary, config).map_err(io_error)?;
    }
    Ok(home)
}

fn critical_fingerprint(distribution: &Distribution, home: &Path) -> Result<String> {
    let mut result = String::new();
    for file in distribution.required_files() {
        result.push_str(&checksum(&home.join(file))?);
        result.push('\n');
    }
    Ok(result)
}

pub fn stage_native(distribution: &Distribution, home: &Path, output: &Path) -> Result<()> {
    if output.exists() {
        fs::remove_dir_all(output).map_err(io_error)?;
    }
    fs::create_dir_all(output).map_err(io_error)?;
    let sources = if distribution.windows() {
        vec![home.to_owned(), home.join("libs")]
    } else {
        vec![home.join("lib")]
    };
    for source in sources {
        for entry in fs::read_dir(source).map_err(io_error)? {
            let entry = entry.map_err(io_error)?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let native = if distribution.windows() {
                name_str.ends_with(".dll") || name_str.ends_with(".lib")
            } else {
                name_str.starts_with("libpython")
                    && (name_str.contains(".so") || name_str.ends_with(".dylib"))
            };
            if native && entry.path().is_file() {
                // Copy symlink targets as ordinary files; Windows needs no symlink privileges.
                fs::copy(entry.path(), output.join(name)).map_err(io_error)?;
            }
        }
    }
    if distribution.windows() {
        // PyO3's suppressed build script retains its generic #[link(name = "pythonXY")].
        fs::copy(
            output.join(format!("{}.lib", distribution.lib_name())),
            output.join("pythonXY.lib"),
        )
        .map_err(io_error)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    fn temp() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "pylink-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn catalog_is_complete_and_pinned() {
        let versions: std::collections::BTreeSet<_> = CATALOG
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
            .map(|line| line.split('\t').next().unwrap())
            .collect();
        for version in versions {
            for target in [
                "aarch64-apple-darwin",
                "x86_64-apple-darwin",
                "aarch64-unknown-linux-gnu",
                "x86_64-unknown-linux-gnu",
                "aarch64-pc-windows-msvc",
                "x86_64-pc-windows-msvc",
            ] {
                let d = Distribution::select(version, target).unwrap();
                assert_eq!(d.sha256.len(), 64);
                assert!(d.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
                assert!(d.url().ends_with("-install_only_stripped.tar.gz"));
                assert_eq!(Distribution::select(d.version, target).unwrap(), d);
                assert!(Distribution::select(d.minor(), target).is_ok());
            }
        }
    }
    #[test]
    fn rejects_unsupported_and_ambiguous_versions() {
        for version in [
            "3",
            "latest",
            "3.14t",
            "3.14.99",
            "../3.14",
            "3.14\n3.13",
            "03.14",
            "3.1",
            "",
        ] {
            assert!(Distribution::select(version, "aarch64-apple-darwin").is_err());
        }
        assert!(Distribution::select("3.14", "x86_64-unknown-linux-musl").is_err());
    }
    #[test]
    fn old_python_versions_explain_the_minimum_and_how_to_select_it() {
        for version in ["2.7", "3.12", "3.12.14", "3.13", "3.13.15"] {
            let error = Distribution::select(version, "aarch64-apple-darwin").unwrap_err();
            assert!(error.contains("requires Python 3.14 or newer"), "{error}");
            assert!(error.contains("PyInitConfig"), "{error}");
            assert!(error.contains("PYLINK_PYTHON_VERSION=3.14"), "{error}");
        }
    }
    #[test]
    fn unavailable_future_versions_report_catalog_versions() {
        let target = "aarch64-apple-darwin";
        let error = Distribution::select("3.999", target).unwrap_err();
        assert!(
            error.contains("not in this crate's pinned catalog"),
            "{error}"
        );
        let default = Distribution::select(DEFAULT_VERSION, target).unwrap();
        assert!(error.contains(default.version), "{error}");
    }
    #[test]
    fn selects_by_target_not_host() {
        let windows = Distribution::select("3.14", "x86_64-pc-windows-msvc").unwrap();
        assert_eq!(windows.lib_name(), "python314");
        assert_eq!(
            windows.include_dir(Path::new("python")),
            Path::new("python/include")
        );
        let linux = Distribution::select("3.14", "aarch64-unknown-linux-gnu").unwrap();
        assert_eq!(linux.lib_name(), "python3.14");
        assert_ne!(windows.cache_key(), linux.cache_key());
    }
    #[test]
    fn system_checksum_matches_known_vectors_and_detects_corruption() {
        let dir = temp();
        let file = dir.join("space ' unicode λ [literal].txt");
        fs::write(&file, b"").unwrap();
        assert_eq!(
            checksum(&file).unwrap(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        fs::write(&file, b"abc").unwrap();
        assert_eq!(
            checksum(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::write(&file, b"abd").unwrap();
        assert_ne!(
            checksum(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        fs::remove_file(&file).unwrap();
        assert!(checksum(&file).is_err(), "missing file must fail hashing");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn offline_cache_miss_does_not_download() {
        let dir = temp();
        let d = Distribution::select("3.14", "aarch64-apple-darwin").unwrap();
        assert!(prepare(&d, &dir, true).unwrap_err().contains("offline"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn lock_is_released_on_close() {
        let dir = temp();
        let path = dir.join("entry.lock");
        let lock = acquire_lock(&path).unwrap();
        let second = File::options().read(true).write(true).open(&path).unwrap();
        assert!(matches!(
            second.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(lock);
        drop(second);
        // Another parallel test may be between fork and exec, briefly retaining
        // the original lock descriptor. Real callers also wait for this lock.
        drop(acquire_lock(&path).unwrap());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn paths_in_cargo_directives_must_be_absolute_and_single_line() {
        assert!(absolute_path(PathBuf::from("relative"), "test").is_err());
        assert!(absolute_path(env::temp_dir().join("bad\npath"), "test").is_err());
    }
}
