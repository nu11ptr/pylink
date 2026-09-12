# Integration test programs

These standalone applications are CI fixtures, not getting-started examples.
For a minimal introduction, see [hello.rs](../../examples/hello.rs) or the
[PyO3 hello example](../../examples/hello-pyo3).

- `basic-app` builds `pylink-basic-smoke-test`, a consumer whose only dependency
  is `pylink`. Its repeated initialization calls check runtime
  discovery and that initialization is safe to call more than once.
- `pyo3-app` builds `pylink-pyo3-smoke-test`. Its assertions check Python
  execution, native module imports, and Python access from multiple Rust threads.

Both programs support `--print-home` for test and packaging automation.
[`scripts/smoke.py`](../../scripts/smoke.py) runs the hello examples and these
fixtures, checks debug and release builds, reuses the download cache offline,
and runs relocated bundles with the build directories and cache hidden. It
also disables C/C++ compiler discovery and PyO3's system-Python discovery.
The GitHub Actions matrix runs these checks on Windows, Linux, and macOS.
