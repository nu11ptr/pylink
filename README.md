# pylink

[![crates.io](https://img.shields.io/crates/v/pylink.svg)](https://crates.io/crates/pylink)
[![docs.rs](https://docs.rs/pylink/badge.svg)](https://docs.rs/pylink)
[![CI](https://github.com/nu11ptr/pylink/actions/workflows/ci.yml/badge.svg)](https://github.com/nu11ptr/pylink/actions/workflows/ci.yml)

Download, cache, link, and initialize CPython in a Rust application using
Astral's stripped [standalone Python builds](https://github.com/astral-sh/python-build-standalone).
No installed Python is needed to build or run your application. pylink has zero
Cargo dependencies and compiles no C/C++ source.

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

## Examples

Run these from a repository checkout:

[Rust hello](https://github.com/nu11ptr/pylink/blob/main/examples/hello.rs)
initializes Python and prints a Rust greeting:

```sh
cargo run --example hello
```

[PyO3 hello](https://github.com/nu11ptr/pylink/tree/main/examples/hello-pyo3)
evaluates a greeting in Python. Run it from its directory so Cargo reads
`.cargo/config.toml`:

```sh
cd examples/hello-pyo3
cargo run
```

## Build requirements and supported targets

- Rust **1.89+**, with its usual linker and SDK.
- Python **3.14+**; downloaded automatically.
- `curl`, `tar`, and a SHA-256 tool:

| Target | Tools / SDK | Python's minimum OS |
| --- | --- | --- |
| `aarch64-apple-darwin` | Xcode Command Line Tools; built-in curl, tar, shasum | macOS 11 |
| `x86_64-apple-darwin` | Same | macOS 10.15 |
| `x86_64-unknown-linux-gnu` | Rust linker toolchain, curl, tar with gzip, sha256sum | glibc 2.17 |
| `aarch64-unknown-linux-gnu` | Same | glibc 2.17 |
| `x86_64-pc-windows-msvc` | MSVC linker/Windows SDK; built-in curl.exe, tar.exe, PowerShell | Windows 10 |
| `aarch64-pc-windows-msvc` | Same, with ARM64 linker/SDK | Windows 10 |

- Debian/Ubuntu tools: `apt-get install curl tar gzip coreutils`.
- macOS toolchain: `xcode-select --install`.

Linux's Rust toolchain may still use `cc` as its linker driver. Python selection
follows Cargo's target; cross-compiling needs the target linker and SDK.

CI covers Linux x64, Windows x64, and macOS ARM64/x64. Linux/Windows ARM64 are
available but not CI-tested. musl, MinGW, 32-bit, and free-threaded Python are
unsupported.

Your toolchain or application may need a newer OS than Python's minimum.
See [upstream requirements](https://github.com/astral-sh/python-build-standalone/blob/20260901/docs/running.rst).

## Choosing Python

The default is **Python 3.14.7** from Astral release **20260901**. Downloads and
SHA-256 hashes are pinned per crate release. The current catalog accepts `3.14`
or `3.14.7`; unsupported versions fail.

Set the version in your application's `.cargo/config.toml`:

```toml
[env]
PYLINK_PYTHON_VERSION = "3.14"
```

Or use a one-line `.python-version` file containing `3.14`:

```toml
[env]
PYLINK_VERSION_FILE = { value = ".python-version", relative = true }
```

`PYLINK_PYTHON_VERSION` takes precedence over the version file, then the default.
Run Cargo from the application/workspace directory to load its configuration.
Changes to the selected version file trigger a rebuild.

## Cache and network behavior

| Build host | Default cache |
| --- | --- |
| macOS | `~/Library/Caches/pylink` |
| Linux | `$XDG_CACHE_HOME/pylink`, or `~/.cache/pylink` |
| Windows | `%LOCALAPPDATA%\pylink\Cache` |

Downloads are cached by version, release, target, and hash, with SHA-256 verified
before extraction. File locking lets concurrent builds share the cache.

On reuse, pylink checks key runtime files and repairs missing or damaged ones
from the cached archive. Delete the entry if other extracted files are damaged.

| Setting | Meaning |
| --- | --- |
| `PYLINK_CACHE_DIR` | Absolute cache directory override |
| `PYLINK_OFFLINE=1` | Forbid Python downloads |
| `CARGO_NET_OFFLINE=true` | Also forbids Python downloads when set in the environment |
| `PYLINK_LINK_MODE=dynamic` | Default and only supported link mode |

For fully offline builds, populate the Cargo and interpreter caches, then use **both**
`cargo --offline` and `PYLINK_OFFLINE=1`.

## Initialization

Call `initialize()` at startup, preferably on the main thread, before using
Python. Repeated and concurrent calls are safe. Python stays initialized until
process exit; do not finalize or independently reinitialize it.

Isolated mode ignores Python environment settings, user site packages, and
application arguments. Initialization leaves signal handlers unchanged,
disables bytecode writes, and releases the GIL.

Python home lookup order:

1. Runtime `PYLINK_PYTHON_HOME`.
2. `python/` beside the executable.
3. `Contents/Resources/python/` for a macOS `.app`.
4. The build-time interpreter cache, for development.

- `initialize_from(path)`: use an explicit Python home.
- `runtime_home()`: get the selected home.

Incomplete bundles return an error. Paths must be valid Unicode; spaces are
supported. Library loading must be configured before launch; see
[shipping](#shipping-an-application).

Build information:

- Constants: `PYTHON_VERSION`, `PYTHON_RELEASE`, `BUILD_PYTHON_HOME`, `PYO3_CONFIG_FILE`.
- Consumer build-script variables: `DEP_PYLINK_PYTHON_PYTHON_HOME`,
  `DEP_PYLINK_PYTHON_PYTHON_VERSION`, `DEP_PYLINK_PYTHON_PYO3_CONFIG_FILE`.

## PyO3

See the complete [hello-world example](https://github.com/nu11ptr/pylink/tree/main/examples/hello-pyo3).

```toml
[dependencies]
pylink = "0.1"
pyo3 = { version = "0.29", default-features = false }
```

```rust
use pyo3::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pylink::initialize()?;
    let greeting = Python::attach(|py| -> PyResult<String> {
        py.eval(c"'Hello from Python!'", None, None)?.extract()
    })?;
    println!("{greeting}");
    Ok(())
}
```

Place `pyo3-config.txt` beside the application's `Cargo.toml`:

```text
implementation=CPython
version=3.14
shared=true
abi3=false
suppress_build_script_link_lines=true
```

Point to it in `.cargo/config.toml`:

```toml
[env]
PYO3_CONFIG_FILE = { value = "pyo3-config.txt", relative = true }
```

This config disables system-Python discovery and leaves linking to pylink.

- Keep the config's Python minor version equal to pylink's selected minor.
- Leave PyO3's `auto-initialize`, `extension-module`, and `abi3` features disabled.

For builds that prepare Python first, pylink also exposes a generated config as
`PYO3_CONFIG_FILE`. See [PyO3 build configuration](https://pyo3.rs/v0.29.2/building-and-distribution.html).

## Shipping an application

`cargo run` and `cargo test` handle development library paths automatically.
For distribution, ship the **entire Python home**, including native extensions
and licenses, for the **same target and Python version** as the binary.
Your users do not need Python installed.

Run [scripts/bundle.py](https://github.com/nu11ptr/pylink/blob/main/scripts/bundle.py)
on the target OS to create the layouts below and configure library loading:

```sh
python3 scripts/bundle.py --binary /path/to/myapp \
  --python-home /path/from/pylink/BUILD_PYTHON_HOME \
  --output dist/myapp
```

Use `python` on Windows; Python is needed only to run the packaging helper.
Obtain the home from `pylink::BUILD_PYTHON_HOME` or build-script metadata, and
choose an output directory that does not already exist.

### Linux

```text
myapp/
  myapp                    # launcher; run this
  bin/myapp                # Rust executable
  python/
    lib/libpython3.14.so.1.0
    lib/python3.14/
    ...                    # rest of the distribution
```

The launcher sets `LD_LIBRARY_PATH` and `PYLINK_PYTHON_HOME`, then runs the binary.

To run without a launcher, place the executable beside `python/` and set its
RUNPATH to `$ORIGIN/python/lib` using the application's `build.rs` or `patchelf`.

### macOS

```text
myapp/
  myapp
  python/
    lib/libpython3.14.dylib
    lib/python3.14/
    ...
```

The helper changes `@rpath/libpython3.14.dylib` to
`@executable_path/python/lib/libpython3.14.dylib` and restores the ad-hoc
signature. Run `myapp` directly.

For `.app` bundles, put the executable in `Contents/MacOS/` and Python in
`Contents/Resources/python/`. Use `install_name_tool -change` to set the
dependency to `@executable_path/../Resources/python/lib/libpython3.14.dylib`.

Apply Developer ID signing/notarization, including nested libraries/extensions,
after packaging.

### Windows

```text
myapp/
  myapp.exe
  python314.dll
  python3.dll
  vcruntime140.dll
  vcruntime140_1.dll        # when present
  python/
    Lib/
    DLLs/
    python314.dll
    ...
```

Keep the full `python/` directory and copy **all its root DLLs** beside the
executable. The helper does both. Run `myapp.exe` directly; no PATH changes are
needed.

### Static linking

Static linking is unsupported because the stripped distributions do not include
static libpython. `PYLINK_LINK_MODE=static` returns an error.

## Tests and maintenance

From a repository checkout:

```sh
cargo test --all-targets
cargo test --doc
python3 scripts/smoke.py
```

- Runs both hello examples and the dedicated [test applications](https://github.com/nu11ptr/pylink/tree/main/tests/fixtures).
- Checks initialization, native module imports, threading, cache integrity, and offline builds.
- Tests builds with C/C++ compiler and system-Python discovery disabled.
- Runs packaged binaries with the original cache/build directories hidden and a minimal environment.
- Exercises bundle paths containing spaces and Unicode.

[GitHub Actions](https://github.com/nu11ptr/pylink/blob/main/.github/workflows/ci.yml)
runs the suite on Windows, Linux, and macOS. CI also checks Rust 1.89, crate
packaging, and documentation without Python downloads.

Update the pinned catalog:

```sh
python3 scripts/update_manifest.py --release 20260901 \
  --versions 3.14.7
```

For a new Python minor, also update the default, PyO3 configs, examples, docs, and CI.

## License

[MIT](https://github.com/nu11ptr/pylink/blob/main/LICENSE-MIT) OR
[Apache-2.0](https://github.com/nu11ptr/pylink/blob/main/LICENSE-APACHE), at your option.
Python and its bundled dependencies retain their own licenses.
