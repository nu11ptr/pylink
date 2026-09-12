# pylink

Download, cache, link, and initialize a redistributable CPython interpreter in a
Rust application. Uses Astral's smallest regular, GIL-enabled
[`install_only_stripped` builds](https://github.com/astral-sh/python-build-standalone).
No system Python installation is needed to build or run your application.

```toml
[dependencies]
pylink = "0.1"
```

```rust
fn main() -> Result<(), pylink::Error> {
    pylink::initialize()?;
    // Use Python through PyO3 or the Python C API.
    Ok(())
}
```

This repository is an initial implementation; the dependency declaration above
assumes publication. For now use a local path or this Git repository.

## Build requirements and supported targets

Rust **1.89+** with its normal native linker/SDK, `curl`, `tar`, and the
platform's SHA-256 utility. pylink has **zero Cargo dependencies**, including
build dependencies, and compiles no C or C++ source.

Rust calls Python 3.14+'s opaque
[`PyInitConfig` API](https://docs.python.org/3.14/c-api/init_config.html#pyinitconfig-c-api)
directly through the C ABI. Python allocates and manages the configuration;
pylink sets options through functions without duplicating Python's struct
layouts. No `bindgen`, `libclang`, or C compiler is needed.

| Target | Build prerequisites | Python's minimum OS |
| --- | --- | --- |
| `aarch64-apple-darwin` | Xcode Command Line Tools; macOS-provided curl, tar, shasum | macOS 11 |
| `x86_64-apple-darwin` | Same | macOS 10.15 |
| `x86_64-unknown-linux-gnu` | Normal Rust linker toolchain, curl, tar with gzip, sha256sum | glibc 2.17 |
| `aarch64-unknown-linux-gnu` | Same | glibc 2.17 |
| `x86_64-pc-windows-msvc` | MSVC linker and Windows SDK (normal Rust setup); Windows curl.exe, tar.exe, PowerShell | Windows 10 for Python 3.14 |
| `aarch64-pc-windows-msvc` | Same, with ARM64 linker/SDK components | Same |

For Debian/Ubuntu, `apt-get install curl tar gzip coreutils` provides the download
and extraction tools. Keep the linker setup you normally use for Rust binaries:
the default GNU Rust toolchain commonly invokes `cc` as its linker driver, even
when compiling only Rust. On macOS, `xcode-select --install` provides the usual
linker and SDK. pylink adds no C compilation requirement to that setup.
Rust, third-party libraries, or your application may require newer OS versions
than Python itself. See [upstream platform requirements](https://github.com/astral-sh/python-build-standalone/blob/20260901/docs/running.rst).

Distribution selection uses Cargo's **TARGET**, never the build host. Native
builds are the tested path. Cross-compilation also needs the target linker and
SDK configured through Cargo; pylink does not install those.
The CI matrix covers Linux x64, Windows x64, and macOS ARM64/x64. Linux and Windows
ARM64 have pinned distributions but are not exercised in CI yet. musl, MinGW,
32-bit, free-threaded Python, and static linking are not supported.

## Choosing Python

Python **3.14+** is required. The default is **Python 3.14.7**, from Astral release
**20260901**. Each crate release pins its downloads and SHA-256 hashes; builds
never query GitHub's "latest" endpoint. Updating pylink updates the available interpreter catalog.
This makes builds reproducible and usable offline after the first download.

Select a supported series or exact patch in your application's
`.cargo/config.toml`:

```toml
[env]
PYLINK_PYTHON_VERSION = "3.14"
```

The current catalog contains `3.14` / `3.14.7` for every target above.
Unsupported versions fail with a diagnostic; they do not silently select a
different patch or contact a mutable release URL.

Alternatively, keep a one-line `.python-version` file containing `3.14`, and
explicitly tell Cargo where to find it:

```toml
[env]
PYLINK_VERSION_FILE = { value = ".python-version", relative = true }
```

The environment variable `PYLINK_PYTHON_VERSION` takes precedence over the
file, which takes precedence over the default. Changes to the selected file
trigger a rebuild. Run Cargo from the application/workspace so it discovers its
`.cargo/config.toml`.

Cargo supports `[package.metadata]`, but it does not pass an application's
metadata or root directory to a dependency's build script. Explicit Cargo
configuration avoids guessing which consumer owns a `.python-version` file.
Version features would also be additive across dependencies and could conflict.

## Cache and network behavior

Default cache locations:

| Build host | Directory |
| --- | --- |
| macOS | `~/Library/Caches/pylink` |
| Linux | `$XDG_CACHE_HOME/pylink`, or `~/.cache/pylink` |
| Windows | `%LOCALAPPDATA%\pylink\Cache` |

Each cache entry is keyed by exact Python version, Astral release, target, and
full expected SHA-256. The original archive and extracted runtime are retained.
Whenever the build script runs, it checks the archive hash, completion marker,
required layout, exact header version, and hashes of the native libraries,
version header, and encoding entry point. Missing or damaged extraction
is rebuilt from the verified cached archive. It does not hash every standard
library file on each build; delete the entry if other extracted files are damaged.

Downloads use HTTPS, a temporary file, retries, and a pinned hash checked before
extraction. An OS file lock serializes concurrent builds; it is released if a
build process dies. Extraction is staged before publication. No administrator
access or global Python installation is used.

Useful settings in `.cargo/config.toml` or the shell:

| Setting | Meaning |
| --- | --- |
| `PYLINK_CACHE_DIR` | Absolute override for the entire cache directory |
| `PYLINK_OFFLINE=1` | Forbid downloads; fail if a verified archive is unavailable |
| `CARGO_NET_OFFLINE=true` | Also forbids pylink downloads when set as an environment variable |
| `PYLINK_LINK_MODE=dynamic` | The only supported link mode; also the default |

For fully offline builds, use **both** `cargo --offline` and
`PYLINK_OFFLINE=1`: Cargo's command-line `--offline` flag is not exposed to
dependency build scripts. Cargo's crate cache must also already be populated.
Keep the interpreter cache while working on applications that use its development
fallback. Cargo detects missing critical cache files on the next build.

## Initialization

`initialize()` starts Python in isolated mode, ignores `PYTHONHOME`, `PYTHONPATH`,
user site packages and the application's command-line arguments, disables Python
signal handler installation and bytecode writes, then releases the GIL. Call it
at startup, preferably on the main thread, **before** another library initializes
Python. Concurrent calls through pylink are serialized and repeated calls are
safe. The interpreter lives until process exit; do not finalize or independently
reinitialize it.

Python home lookup, in order:

1. Runtime `PYLINK_PYTHON_HOME` override.
2. A `python/` directory beside the application executable.
3. `Contents/Resources/python/` for a macOS `Contents/MacOS/` executable.
4. The downloaded build-time cache, for development.

An existing but incomplete bundle returns an error. It is not silently replaced
with a cache or system interpreter. `initialize_from(path)` selects an explicit
home; `runtime_home()` returns the selected directory. These select the standard
library **after** the OS loads the executable. Shared library discovery must be
configured before launch, as described below.

The Python home and application executable paths must be valid Unicode because
`PyInitConfig` accepts UTF-8 strings. Spaces and Unicode characters are supported.
Non-UTF-8 paths on Unix and unpaired UTF-16 surrogates on Windows return an error
instead of being converted lossily.

`PYTHON_VERSION`, `PYTHON_RELEASE`, `BUILD_PYTHON_HOME`, and `PYO3_CONFIG_FILE`
expose build-time information. The build script also supplies
`DEP_PYLINK_PYTHON_PYTHON_HOME`, `DEP_PYLINK_PYTHON_PYTHON_VERSION`, and
`DEP_PYLINK_PYTHON_PYO3_CONFIG_FILE` to an immediate consumer's build script.

## PyO3

PyO3 is not a pylink dependency. The complete standalone application in
[`examples/pyo3-app`](examples/pyo3-app) demonstrates integration:

```toml
[dependencies]
pylink = "0.1"
pyo3 = { version = "0.29", default-features = false }
```

```rust
use pyo3::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pylink::initialize()?;
    Python::attach(|py| -> PyResult<()> {
        let sys = py.import("sys")?;
        println!("{}", sys.getattr("version")?);
        Ok(())
    })?;
    Ok(())
}
```

Check in `pyo3-config.txt` beside the application's Cargo.toml:

```text
implementation=CPython
version=3.14
shared=true
abi3=false
suppress_build_script_link_lines=true
```

Then point PyO3 to it in `.cargo/config.toml`:

```toml
[env]
PYO3_CONFIG_FILE = { value = "pyo3-config.txt", relative = true }
```

**Keep the PyO3 config's minor version equal to pylink's selected minor.**
This disables system-interpreter discovery and lets pylink supply the native
link. Do not enable PyO3's `auto-initialize`, `extension-module`, or `abi3`
features with this full-API example. The Windows build also supplies the generic
`pythonXY.lib` alias used by PyO3 with suppressed link configuration.

PyO3 and pylink have independent build scripts: pylink cannot set an
environment variable for its sibling before it builds. A checked-in config
solves that ordering problem. pylink additionally generates an exact config in
each cache entry and exposes its path as `PYO3_CONFIG_FILE` for workflows that
prepare Python first and build the consumer afterward. See
[PyO3 build configuration](https://pyo3.rs/v0.29.2/building-and-distribution.html).

## Shipping an application

`cargo run` and `cargo test` work without setting loader paths: pylink stages
native libraries in Cargo's build output and emits the native link instructions.
For distribution, ship the **entire** selected Python home, including its native
extensions, data, and license notices. A stripped interpreter still needs its
standard library. This implementation does not produce a single-file executable.

Use the stdlib-only helper on the target OS (Python is needed for packaging,
not by your users):

```sh
python3 scripts/bundle.py --binary /path/to/myapp \
  --python-home /path/from/pylink/BUILD_PYTHON_HOME \
  --output dist/myapp
```

On Windows use `python` instead of `python3`. The output directory must not
already exist. To see a home path in this repository, run
`cargo run --example basic -- --print-home`. For your own application, expose
`pylink::BUILD_PYTHON_HOME` in your packaging tooling or read the build metadata.
The binary and Python home must come from the **same target and version**.

### Linux

```text
myapp/
  myapp                    # shell launcher; users run this
  bin/myapp                # actual Rust executable
  python/
    lib/libpython3.14.so.1.0
    lib/python3.14/         # stdlib, lib-dynload, site-packages
    ...                    # rest of the distribution
```

The helper's launcher sets `LD_LIBRARY_PATH` to `python/lib` and
`PYLINK_PYTHON_HOME` to `python`, then executes `bin/myapp`. It handles paths
containing spaces and preserves application arguments.

To avoid a launcher, arrange `myapp` beside `python/` and add an application
`build.rs` emitting `cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/python/lib`, or patch
the release binary's RUNPATH with `patchelf`. A library crate's
`rustc-link-arg` **does not propagate** to downstream executables; see
[Cargo's build-script rules](https://doc.rust-lang.org/cargo/reference/build-scripts.html).

### macOS

```text
myapp/
  myapp
  python/
    lib/libpython3.14.dylib
    lib/python3.14/
    ...
```

The helper rewrites the executable's Python dependency from
`@rpath/libpython3.14.dylib` to
`@executable_path/python/lib/libpython3.14.dylib`, then restores its ad-hoc
signature. Run the executable directly.

For a GUI `.app`, put the executable in `MyApp.app/Contents/MacOS/` and the Python
home in `MyApp.app/Contents/Resources/python/`. Rewrite the dependency to
`@executable_path/../Resources/python/lib/libpython3.14.dylib` with
`install_name_tool -change` before signing. Runtime discovery supports this
layout. Apply your normal Developer ID signing/notarization process, including
nested Python libraries/extensions, after all packaging changes.

### Windows

```text
myapp/
  myapp.exe
  python314.dll
  python3.dll
  vcruntime140.dll          # copy all DLLs from Python's root here
  vcruntime140_1.dll        # when present in that target's distribution
  python/
    Lib/
    DLLs/
    python314.dll
    ...
```

Keep the full Python directory and copy its root `*.dll` files beside the Rust
executable. The Windows loader needs those DLLs before `main()` runs. The helper
does both, including runtime DLLs present in the selected archive. Run
`myapp.exe` directly; no Python installation or PATH edits are needed.

### Static linking

Static linking is intentionally not offered yet. The compact stripped archives
contain no static libpython, and the pinned release has no Windows static
builds. Supporting static embedding needs different distributions, additional
native dependencies and toolchain handling; see
[upstream embedding notes](https://github.com/astral-sh/python-build-standalone/blob/20260901/docs/running.rst).
Selecting `PYLINK_LINK_MODE=static` returns an explicit error.

## Tests and maintenance

```sh
cargo test --all-targets
cargo test --doc
python3 scripts/smoke.py
```

The smoke test builds a [dependency-only consumer](examples/basic-app) and a
real PyO3 consumer with system-interpreter discovery
pointing at a nonexistent file; imports native modules including SSL, SQLite,
ctypes and zlib; attaches another Rust thread; rebuilds in a fresh target
directory offline; and runs both application bundles after moving the cache and
build directories out of reach. Bundles have spaces and Unicode characters in
their paths and run with minimal environments. Unit/integration tests also
exercise initialization errors, version/target selection, integrity, offline
cache repair, and locking.

[GitHub Actions](.github/workflows/ci.yml) runs Python 3.14 on Windows,
Linux, and both macOS architectures. Consumer tests set C/C++ compiler settings
to nonexistent executables and build from fresh target directories. They also
assert that pylink has no Cargo dependencies. Runtime tests exercise the opaque
initialization API directly; there are no copied configuration layouts to check.
The workflow runs on push, pull request, and manual dispatch. Local verification
on one OS does not substitute for those other runners.

To update the pinned catalog deliberately:

```sh
python3 scripts/update_manifest.py --release 20260901 \
  --versions 3.14.7
```

The updater cross-checks GitHub asset digests against the release's SHA256SUMS
before replacing `build_support/distributions.tsv`. Update the default, PyO3
example/config, documentation and CI as needed when adding a new Python minor.
Review upstream licenses and release notes when changing distributions.
