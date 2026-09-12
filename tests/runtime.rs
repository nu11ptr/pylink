use std::ffi::{CString, c_char, c_int};

unsafe extern "C" {
    fn PyGILState_Ensure() -> c_int;
    fn PyGILState_Release(state: c_int);
    fn PyRun_SimpleString(code: *const c_char) -> c_int;
}

fn run_python(code: &str) {
    let code = CString::new(code).unwrap();
    // SAFETY: pybundle initialized CPython, and this scope acquires and releases
    // the GIL on the same thread around the only Python C API operation.
    let status = unsafe {
        let state = PyGILState_Ensure();
        let status = PyRun_SimpleString(code.as_ptr());
        PyGILState_Release(state);
        status
    };
    assert_eq!(status, 0, "Python script failed (see traceback above)");
}

fn incomplete_python_home(name: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("pybundle-{name}-{}", std::process::id()));
    let minor_version = pybundle::PYTHON_VERSION
        .split('.')
        .take(2)
        .collect::<Vec<_>>()
        .join(".");
    let stdlib = if cfg!(windows) {
        home.join("Lib")
    } else {
        home.join("lib").join(format!("python{minor_version}"))
    };
    let encodings = stdlib.join("encodings");
    std::fs::create_dir_all(&encodings).unwrap();
    std::fs::write(encodings.join("__init__.py"), "").unwrap();
    home
}

#[test]
fn initializes_isolated_python_and_releases_the_gil_for_other_threads() {
    let home = pybundle::runtime_home().unwrap();
    assert!(matches!(
        pybundle::initialize_from("bad\0path"),
        Err(pybundle::Error::InvalidPath(_))
    ));
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        assert!(matches!(
            pybundle::initialize_from(std::ffi::OsStr::from_bytes(b"bad\xffpath")),
            Err(pybundle::Error::NonUnicodePath(_))
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        assert!(matches!(
            pybundle::initialize_from(std::ffi::OsString::from_wide(&[0xd800])),
            Err(pybundle::Error::NonUnicodePath(_))
        ));
    }
    let missing = home.join("this-python-home-does-not-exist");
    assert!(matches!(
        pybundle::initialize_from(missing),
        Err(pybundle::Error::InvalidHome { .. })
    ));
    pybundle::initialize().unwrap();
    pybundle::initialize().unwrap();
    pybundle::initialize_from(&home).unwrap();
    assert_eq!(pybundle::runtime_home().unwrap(), home);
    let other_home = incomplete_python_home("other-python-home");
    assert!(matches!(
        pybundle::initialize_from(&other_home),
        Err(pybundle::Error::AlreadyInitialized { .. })
    ));
    std::fs::remove_dir_all(other_home).unwrap();
    run_python(&format!(
        "import sys, json, ssl, sqlite3, ctypes, zlib, hashlib, subprocess\n\
         assert sys.version.split()[0] == {:?}\n\
         assert sys.flags.isolated == 1\n\
         assert sys.flags.ignore_environment == 1\n\
         assert sys.flags.no_user_site == 1\n\
         assert sys.flags.safe_path\n\
         assert sys.flags.utf8_mode == 1\n\
         assert sys.dont_write_bytecode\n\
         assert json.loads('{{\"answer\": 42}}')['answer'] == 42\n\
         assert sqlite3.connect(':memory:').execute('select 42').fetchone()[0] == 42\n\
         assert zlib.decompress(zlib.compress(b'hello')) == b'hello'\n\
         assert hashlib.sha256(b'hello').hexdigest().startswith('2cf24dba')\n\
         assert ssl.OPENSSL_VERSION\n",
        pybundle::PYTHON_VERSION
    ));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                pybundle::initialize().unwrap();
                run_python("assert sum(range(10)) == 45");
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn initialization_ignores_python_environment_settings() {
    const CHILD_MARKER: &str = "PYBUNDLE_TEST_ISOLATED_ENVIRONMENT";
    const IGNORED_PATH: &str = "pybundle-pythonpath-must-be-ignored";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let home = pybundle::runtime_home().unwrap();
        let executable = std::env::current_exe().unwrap();
        pybundle::initialize().unwrap();
        run_python(&format!(
            "import sys\n\
             assert sys.flags.utf8_mode == 1\n\
             assert sys.flags.ignore_environment == 1\n\
             assert all({IGNORED_PATH:?} not in path for path in sys.path)\n\
             assert sys.prefix == {:?}\n\
             assert sys.executable == {:?}\n",
            home.to_str().unwrap(),
            executable.to_str().unwrap(),
        ));
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "initialization_ignores_python_environment_settings",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .env("PYTHONHOME", "pybundle-invalid-python-home")
        .env("PYTHONPATH", IGNORED_PATH)
        .env("PYTHONUTF8", "0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn initialization_failure_returns_an_error_without_exiting_the_process() {
    const CHILD_MARKER: &str = "PYBUNDLE_TEST_FAILED_INITIALIZATION";
    if std::env::var_os(CHILD_MARKER).is_some() {
        // Satisfy the inexpensive directory check but deliberately omit the
        // actual codecs, making CPython's initialization return a config error.
        let home = incomplete_python_home("incomplete-python");
        let first = pybundle::initialize_from(&home).unwrap_err();
        assert!(matches!(first, pybundle::Error::Initialization(_)));
        assert_eq!(pybundle::initialize_from(&home).unwrap_err(), first);
        std::fs::remove_dir_all(home).unwrap();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "initialization_failure_returns_an_error_without_exiting_the_process",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
