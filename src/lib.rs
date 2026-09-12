//! Download, link, and initialize an application-local CPython interpreter.
//!
//! Call [`initialize`] once during application startup, before using PyO3 or
//! another Python API. Python runs in isolated mode: it ignores `PYTHONHOME`,
//! `PYTHONPATH`, the user site directory, and application command-line arguments.
//! Initialization leaves the GIL released so PyO3 can attach Rust threads.
//!
//! ```no_run
//! fn main() -> Result<(), pybundle::Error> {
//!     pybundle::initialize()?;
//!     // Now use pyo3::Python::attach(...), or another Python C API wrapper.
//!     Ok(())
//! }
//! ```
//!
//! Python remains initialized for the lifetime of the process. Do not call
//! `Py_FinalizeEx` or independently initialize Python while using this crate.
//! Prefer initializing on the main thread, which CPython treats as its main
//! interpreter thread.

#![deny(unsafe_op_in_unsafe_fn)]

use std::ffi::{CStr, c_char, c_int};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The exact CPython version selected at build time.
pub const PYTHON_VERSION: &str = env!("PYBUNDLE_PYTHON_VERSION");

/// The pinned Astral python-build-standalone release tag selected at build time.
pub const PYTHON_RELEASE: &str = env!("PYBUNDLE_PYTHON_RELEASE");

/// The generated PyO3 configuration file on the build machine.
///
/// For a two-phase build, first build a small program depending on `pybundle`
/// and have it print this path. Then set the `PYO3_CONFIG_FILE` environment
/// variable to that absolute path before invoking Cargo to build the PyO3
/// application. The generated configuration selects this interpreter's version
/// and leaves native linking to `pybundle`.
///
/// A dependency's build script cannot configure a sibling dependency's build
/// script in the same Cargo invocation. Alternatively, check a matching PyO3
/// configuration into the application repository, as the PyO3 example does.
pub const PYO3_CONFIG_FILE: &str = env!("PYBUNDLE_PYO3_CONFIG_FILE");

/// The downloaded interpreter's home on the build machine.
///
/// This is a development fallback, not a path to use when distributing an app.
/// Bundle its contents in a `python` directory beside the executable, or use
/// `Contents/Resources/python` in a macOS application bundle.
pub const BUILD_PYTHON_HOME: &str = env!("PYBUNDLE_PYTHON_HOME");

#[cfg(windows)]
type WideChar = u16;
#[cfg(not(windows))]
type WideChar = i32;

unsafe extern "C" {
    fn pybundle_initialize_interpreter(
        home: *const WideChar,
        executable: *const WideChar,
        error: *mut c_char,
        capacity: usize,
    ) -> c_int;
}

static INITIALIZATION: OnceLock<Result<PathBuf, Error>> = OnceLock::new();

/// An interpreter discovery or initialization error.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A requested Python home is missing or does not contain its standard library.
    InvalidHome { path: PathBuf, reason: String },
    /// An operating-system path contains an embedded null character.
    InvalidPath(PathBuf),
    /// The current executable's path could not be determined.
    CurrentExecutable(String),
    /// CPython returned an initialization error.
    Initialization(String),
    /// This process already selected another Python home.
    AlreadyInitialized {
        initialized_home: PathBuf,
        requested_home: PathBuf,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHome { path, reason } => {
                write!(
                    formatter,
                    "invalid Python home {}: {reason}",
                    path.display()
                )
            }
            Self::InvalidPath(path) => {
                write!(
                    formatter,
                    "path contains a null character: {}",
                    path.display()
                )
            }
            Self::CurrentExecutable(reason) => {
                write!(formatter, "cannot locate the current executable: {reason}")
            }
            Self::Initialization(reason) => {
                write!(formatter, "cannot initialize CPython: {reason}")
            }
            Self::AlreadyInitialized {
                initialized_home,
                requested_home,
            } => write!(
                formatter,
                "CPython already uses {}; cannot switch to {}",
                initialized_home.display(),
                requested_home.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Locate the Python home, or return the home already used for initialization.
///
/// Before initialization, the search order is:
///
/// 1. The runtime `PYBUNDLE_PYTHON_HOME` environment variable.
/// 2. A `python` directory beside the executable.
/// 3. `../Resources/python` for a macOS `Contents/MacOS` executable.
/// 4. [`BUILD_PYTHON_HOME`], the build machine's downloaded interpreter.
///
/// An explicit override or an existing bundle must be valid; an invalid bundle
/// produces an error instead of silently falling back to the build cache.
pub fn runtime_home() -> Result<PathBuf, Error> {
    if let Some(result) = INITIALIZATION.get() {
        return result.clone();
    }
    let executable = current_executable()?;
    let override_home = std::env::var_os("PYBUNDLE_PYTHON_HOME").map(PathBuf::from);
    resolve_home(override_home, &executable, Path::new(BUILD_PYTHON_HOME))
}

/// Initialize CPython using [`runtime_home`].
///
/// Calls through this crate are serialized and repeated calls are harmless.
/// Initialization does not install signal handlers or write bytecode into the
/// bundle. Its GIL is released before this function returns.
///
/// Call this before any other library initializes CPython. Once CPython has
/// attempted initialization, any initialization failure is remembered: retrying
/// a partially initialized interpreter is not supported. Path validation errors
/// before the CPython call can be corrected and retried.
pub fn initialize() -> Result<(), Error> {
    if let Some(result) = INITIALIZATION.get() {
        return result.as_ref().map(|_| ()).map_err(Clone::clone);
    }
    initialize_from(runtime_home()?)
}

/// Initialize CPython with an explicit interpreter home.
///
/// Relative paths are resolved against the current working directory. The home
/// must contain the standard library from the downloaded interpreter selected at
/// build time. Calling this again with a different home returns an error.
///
/// Homes prepared by this crate contain a `.pybundle-version` file, which must
/// match [`PYTHON_VERSION`] exactly. Manually prepared homes without this marker
/// are accepted; the caller must ensure their standard library and native
/// extensions match the linked interpreter's version.
///
/// This sets Python's module paths. The operating-system loader must already be
/// able to locate the matching Python shared library when the executable starts.
pub fn initialize_from(home: impl AsRef<Path>) -> Result<(), Error> {
    let home = validate_home(home.as_ref())?;
    let result =
        INITIALIZATION.get_or_init(|| initialize_interpreter(&home).map(|()| home.clone()));
    match result {
        Ok(initialized_home) if initialized_home == &home => Ok(()),
        Ok(initialized_home) => Err(Error::AlreadyInitialized {
            initialized_home: initialized_home.clone(),
            requested_home: home,
        }),
        Err(error) => Err(error.clone()),
    }
}

fn current_executable() -> Result<PathBuf, Error> {
    std::env::current_exe().map_err(|error| Error::CurrentExecutable(error.to_string()))
}

fn resolve_home(
    override_home: Option<PathBuf>,
    executable: &Path,
    build_home: &Path,
) -> Result<PathBuf, Error> {
    if let Some(home) = override_home {
        return validate_home(&home);
    }
    if let Some(parent) = executable.parent() {
        let adjacent = parent.join("python");
        if path_is_present(&adjacent) {
            return validate_home(&adjacent);
        }
        #[cfg(target_os = "macos")]
        if parent.file_name().is_some_and(|name| name == "MacOS")
            && let Some(contents) = parent.parent()
        {
            let resources = contents.join("Resources/python");
            if path_is_present(&resources) {
                return validate_home(&resources);
            }
        }
    }
    validate_home(build_home)
}

fn path_is_present(path: &Path) -> bool {
    // A dangling symlink is a broken bundle, not an absent bundle. Preserve
    // other filesystem errors for validate_home to report instead of falling back.
    match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn standard_library(home: &Path) -> PathBuf {
    if cfg!(windows) {
        home.join("Lib")
    } else {
        let mut version = PYTHON_VERSION.split('.');
        let major = version.next().expect("build-time Python major version");
        let minor = version.next().expect("build-time Python minor version");
        home.join("lib").join(format!("python{major}.{minor}"))
    }
}

fn validate_home(home: &Path) -> Result<PathBuf, Error> {
    // Validate before canonicalization so nulls have a useful, platform-neutral error.
    let _ = wide_path(home)?;
    let resolved = home.canonicalize().map_err(|error| Error::InvalidHome {
        path: home.to_path_buf(),
        reason: error.to_string(),
    })?;
    let version_marker = resolved.join(".pybundle-version");
    match std::fs::read_to_string(&version_marker) {
        Ok(version) if version.trim() == PYTHON_VERSION => {}
        Ok(version) => {
            return Err(Error::InvalidHome {
                path: resolved,
                reason: format!(
                    ".pybundle-version specifies Python {:?}; expected {PYTHON_VERSION}",
                    version.trim()
                ),
            });
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && !path_is_present(&version_marker) => {}
        Err(error) => {
            return Err(Error::InvalidHome {
                path: resolved,
                reason: format!("cannot read .pybundle-version: {error}"),
            });
        }
    }
    if !standard_library(&resolved)
        .join("encodings")
        .join("__init__.py")
        .is_file()
    {
        return Err(Error::InvalidHome {
            path: resolved,
            reason: format!(
                "missing the Python {PYTHON_VERSION} standard library (encodings/__init__.py)"
            ),
        });
    }
    Ok(resolved)
}

fn initialize_interpreter(home: &Path) -> Result<(), Error> {
    let home = wide_path(home)?;
    let executable = wide_path(&current_executable()?)?;
    let mut error = [0 as c_char; 2048];
    // SAFETY: the C shim is built against the selected CPython headers. Both
    // strings are terminated and remain alive during the call, and the error
    // buffer has the declared capacity. INITIALIZATION serializes this call.
    let result = unsafe {
        pybundle_initialize_interpreter(
            home.as_ptr(),
            executable.as_ptr(),
            error.as_mut_ptr(),
            error.len(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // SAFETY: the buffer starts zeroed and snprintf in the C shim preserves
        // a final null, including when a diagnostic exceeds its capacity.
        let message = unsafe { CStr::from_ptr(error.as_ptr()) };
        Err(Error::Initialization(
            message.to_string_lossy().into_owned(),
        ))
    }
}

fn wide_path(path: &Path) -> Result<Vec<WideChar>, Error> {
    #[cfg(windows)]
    let mut wide: Vec<WideChar> = {
        use std::os::windows::ffi::OsStrExt;
        path.as_os_str().encode_wide().collect()
    };
    #[cfg(unix)]
    let mut wide = {
        use std::os::unix::ffi::OsStrExt;
        decode_filesystem_path(path.as_os_str().as_bytes())
    };
    if wide.contains(&0) {
        return Err(Error::InvalidPath(path.to_path_buf()));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(unix)]
fn decode_filesystem_path(mut bytes: &[u8]) -> Vec<WideChar> {
    let mut wide = Vec::with_capacity(bytes.len() + 1);
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                wide.extend(valid.chars().map(|character| character as WideChar));
                break;
            }
            Err(error) => {
                let (valid, rest) = bytes.split_at(error.valid_up_to());
                // SAFETY: Utf8Error identifies the prefix as valid UTF-8.
                let valid = unsafe { std::str::from_utf8_unchecked(valid) };
                wide.extend(valid.chars().map(|character| character as WideChar));
                let invalid_length = error.error_len().unwrap_or(rest.len());
                wide.extend(
                    rest[..invalid_length]
                        .iter()
                        .map(|byte| 0xdc00 + i32::from(*byte)),
                );
                bytes = &rest[invalid_length..];
            }
        }
    }
    wide
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "pybundle-runtime-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn python_home(&self, relative: &str) -> PathBuf {
            let home = self.0.join(relative);
            let encodings = standard_library(&home).join("encodings");
            std::fs::create_dir_all(&encodings).unwrap();
            std::fs::write(encodings.join("__init__.py"), "").unwrap();
            home.canonicalize().unwrap()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_prefers_override_then_bundle_then_cache() {
        let temp = TempDir::new();
        let cache = temp.python_home("cache");
        let executable = temp.0.join("app/bin");
        assert_eq!(resolve_home(None, &executable, &cache).unwrap(), cache);
        let adjacent = temp.python_home("app/python");
        assert_eq!(resolve_home(None, &executable, &cache).unwrap(), adjacent);
        let explicit = temp.python_home("custom");
        assert_eq!(
            resolve_home(Some(explicit.clone()), &executable, &cache).unwrap(),
            explicit
        );
        assert!(resolve_home(Some(temp.0.join("missing")), &executable, &cache).is_err());
    }

    #[test]
    fn invalid_bundle_does_not_fall_back_to_cache() {
        let temp = TempDir::new();
        let cache = temp.python_home("cache");
        std::fs::create_dir_all(temp.0.join("app/python")).unwrap();
        assert!(matches!(
            resolve_home(None, &temp.0.join("app/bin"), &cache),
            Err(Error::InvalidHome { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn broken_bundle_symlink_does_not_fall_back_to_cache() {
        let temp = TempDir::new();
        let cache = temp.python_home("cache");
        std::os::unix::fs::symlink(temp.0.join("missing"), temp.0.join("python")).unwrap();
        assert!(matches!(
            resolve_home(None, &temp.0.join("app"), &cache),
            Err(Error::InvalidHome { .. })
        ));
    }

    #[test]
    fn version_marker_must_match_the_exact_selected_version() {
        let temp = TempDir::new();
        let home = temp.python_home("python");
        let marker = home.join(".pybundle-version");
        // A manually prepared home without a marker remains supported.
        assert_eq!(validate_home(&home).unwrap(), home);
        std::fs::write(&marker, format!("{PYTHON_VERSION}\n")).unwrap();
        assert_eq!(validate_home(&home).unwrap(), home);
        let version_with_wrong_patch =
            format!("{}.99999", PYTHON_VERSION.rsplit_once('.').unwrap().0);
        std::fs::write(&marker, &version_with_wrong_patch).unwrap();
        let error = validate_home(&home).unwrap_err();
        assert!(matches!(error, Error::InvalidHome { .. }));
        assert!(error.to_string().contains(&version_with_wrong_patch));
        assert!(error.to_string().contains(PYTHON_VERSION));
        std::fs::write(&marker, [0xff]).unwrap();
        assert!(validate_home(&home).is_err());
        #[cfg(unix)]
        {
            std::fs::remove_file(&marker).unwrap();
            std::os::unix::fs::symlink(home.join("missing"), &marker).unwrap();
            assert!(validate_home(&home).is_err());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn discovers_macos_resources_bundle() {
        let temp = TempDir::new();
        let resources = temp.python_home("App.app/Contents/Resources/python");
        assert_eq!(
            resolve_home(
                None,
                &temp.0.join("App.app/Contents/MacOS/app"),
                &temp.0.join("missing")
            )
            .unwrap(),
            resources
        );
    }

    #[test]
    fn encodes_unicode_and_rejects_nulls() {
        let encoded = wide_path(Path::new("Python-\u{03bb}-\u{1f40d}")).unwrap();
        assert_eq!(encoded.last(), Some(&0));
        #[cfg(unix)]
        assert_eq!(encoded[7], 0x3bb);
        assert!(matches!(
            wide_path(Path::new("bad\0path")),
            Err(Error::InvalidPath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_unix_paths_with_surrogateescape() {
        use std::os::unix::ffi::OsStrExt;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"x\xff\xe2\x82y"));
        assert_eq!(
            wide_path(path).unwrap(),
            [120, 0xdcff, 0xdce2, 0xdc82, 121, 0]
        );
    }
}
