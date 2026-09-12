//! CI integration test application; run through scripts/smoke.py.

use pyo3::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--print-home") {
        println!("{}", pylink::BUILD_PYTHON_HOME);
        return Ok(());
    }

    // Initialize before the first PyO3 call; do not enable auto-initialize.
    pylink::initialize()?;
    Python::attach(|py| -> PyResult<()> {
        let sys = py.import("sys")?;
        let version: String = sys.getattr("version")?.extract()?;
        assert!(version.starts_with(pylink::PYTHON_VERSION));
        let prefix: String = sys.getattr("prefix")?.extract()?;
        assert_eq!(
            std::fs::canonicalize(prefix).expect("Python prefix exists"),
            pylink::runtime_home().expect("selected Python home")
        );

        let value: u32 = py.eval(c"sum(range(10))", None, None)?.extract()?;
        assert_eq!(value, 45);

        // Exercise both pure Python modules and native extension dependencies.
        for module in ["json", "ssl", "sqlite3", "zlib", "ctypes", "hashlib"] {
            py.import(module)?;
        }
        let sqlite = py.import("sqlite3")?;
        let connection = sqlite.call_method1("connect", (":memory:",))?;
        let row: (u32,) = connection
            .call_method1("execute", ("SELECT 40 + 2",))?
            .call_method0("fetchone")?
            .extract()?;
        assert_eq!(row.0, 42);
        println!("Python {version}: sum = {value}, SQLite = {}", row.0);
        Ok(())
    })?;

    // Initialization must release the GIL so another Rust thread can attach.
    std::thread::spawn(|| {
        pylink::initialize().expect("repeat initialization from a second thread");
        Python::attach(|py| {
            let result: u32 = py.eval(c"6 * 7", None, None).unwrap().extract().unwrap();
            assert_eq!(result, 42);
        });
    })
    .join()
    .expect("Python worker thread");
    Ok(())
}
