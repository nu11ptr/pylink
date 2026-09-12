# Hello from Python

From this directory, run:

```sh
cargo run
```

The application initializes Python once, evaluates a greeting through PyO3,
and prints `Hello from Python!`.

The included `.cargo/config.toml` points PyO3 at `pyo3-config.txt`, so no system
Python installation is needed. Run Cargo from this directory to load that
configuration. Its Python minor version must match the version selected by
pylink; both currently use Python 3.14.
