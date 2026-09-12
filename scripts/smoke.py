#!/usr/bin/env python3
"""Build real consumers, then run their relocated bundles without build caches.

Python is only the test/packaging driver. C/C++ compiler configuration and PyO3
interpreter discovery are poisoned, and the packaged applications run with a
separate, minimal environment. The normal Rust native linker remains available.
"""

import contextlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
BASIC_FIXTURE = ROOT / "examples" / "basic-app"
PYO3_FIXTURE = ROOT / "examples" / "pyo3-app"
SUFFIX = ".exe" if os.name == "nt" else ""


def run(args, *, cwd=ROOT, env, capture=False):
    args = [str(arg) for arg in args]
    print("+", " ".join(args), flush=True)
    result = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        check=True,
        text=True,
        stdout=subprocess.PIPE if capture else None,
        timeout=600,
    )
    return result.stdout.strip() if capture else None


@contextlib.contextmanager
def hidden_directories(paths):
    """Make original absolute cache/build paths unavailable, then restore them."""
    renamed = []
    try:
        for original in paths:
            hidden = original.with_name(original.name + f".hidden-{os.getpid()}")
            original.rename(hidden)
            renamed.append((original, hidden))
        yield
    finally:
        for original, hidden in reversed(renamed):
            hidden.rename(original)


def main():
    with tempfile.TemporaryDirectory(prefix="pybundle-smoke-") as temporary:
        work = Path(temporary)
        target = work / "target"
        offline_target = work / "offline-target"
        cache = Path(os.environ.get("PYBUNDLE_CACHE_DIR", work / "cache")).resolve()
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("PYO3_", "PYTHON", "PYBUNDLE_"))
        }
        env.update(
            CARGO_TARGET_DIR=str(target),
            PYBUNDLE_CACHE_DIR=str(cache),
            PYO3_PYTHON=str(work / "there-is-no-system-python"),
        )
        if os.name == "nt":
            # Hashing must work without PowerShell modules. In particular, the
            # runner's PowerShell 7 module paths can break Get-FileHash when
            # inherited by Windows PowerShell through Python and Cargo.
            env["PSMODULEPATH"] = str(work / "there-are-no-powershell-modules")
        selected_version = os.environ.get("PYBUNDLE_PYTHON_VERSION")
        if selected_version:
            env["PYBUNDLE_PYTHON_VERSION"] = selected_version

        # Cargo/rustc do not need these to link Rust binaries, but cc and other
        # C/C++ build helpers honor them. Fresh target directories ensure that
        # a previously compiled C shim cannot make this test pass accidentally.
        rustc_details = run(["rustc", "--version", "--verbose"], env=env, capture=True)
        host = next(
            line.removeprefix("host: ")
            for line in rustc_details.splitlines()
            if line.startswith("host: ")
        )
        missing_compiler = str(work / "there-is-no-c-compiler")
        for compiler in ("CC", "CXX"):
            for name in (
                compiler,
                f"HOST_{compiler}",
                f"TARGET_{compiler}",
                f"{compiler}_{host}",
                f"{compiler}_{host.replace('-', '_')}",
            ):
                env[name] = missing_compiler

        metadata = json.loads(
            run(
                ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
                env=env,
                capture=True,
            )
        )
        package = next(
            package for package in metadata["packages"] if package["name"] == "pybundle"
        )
        assert not package["dependencies"], package["dependencies"]

        run(["cargo", "test", "--locked", "--all-targets"], env=env)
        # Test an independent binary crate whose only dependency is pybundle.
        run(["cargo", "run", "--locked"], cwd=BASIC_FIXTURE, env=env)
        run(["cargo", "run", "--locked", "--release"], cwd=BASIC_FIXTURE, env=env)
        python_home = Path(
            run(
                ["cargo", "run", "--locked", "--quiet", "--release", "--", "--print-home"],
                cwd=BASIC_FIXTURE,
                env=env,
                capture=True,
            )
        )
        assert python_home.is_dir(), python_home

        # Match the selected interpreter, including future catalog additions.
        # Environment settings take priority over the fixture's portable
        # .cargo configuration.
        minor = (python_home / ".pybundle-version").read_text().strip().rsplit(".", 1)[0]
        pyo3_config = work / "pyo3-config.txt"
        pyo3_config.write_text(
            "implementation=CPython\n"
            f"version={minor}\n"
            "shared=true\nabi3=false\nsuppress_build_script_link_lines=true\n"
        )
        pyo3_env = dict(env, PYO3_CONFIG_FILE=str(pyo3_config))
        run(["cargo", "run", "--locked"], cwd=PYO3_FIXTURE, env=pyo3_env)
        run(["cargo", "run", "--locked", "--release"], cwd=PYO3_FIXTURE, env=pyo3_env)

        # A fresh target directory forces the build script to run again. Both
        # Cargo and pybundle must satisfy this build from their existing caches.
        offline_env = dict(env, CARGO_TARGET_DIR=str(offline_target), PYBUNDLE_OFFLINE="1")
        run(["cargo", "run", "--locked", "--offline"], cwd=BASIC_FIXTURE, env=offline_env)

        binaries = [
            target / "release" / f"pybundle-basic-example{SUFFIX}",
            target / "release" / f"pybundle-pyo3-example{SUFFIX}",
        ]
        bundles = []
        for binary in binaries:
            # Spaces exercise launcher quoting; Unicode exercises PyInitConfig's
            # UTF-8 path setters, including their conversion on Windows.
            destination = work / f"relocated bundle café {binary.stem}"
            run(
                [
                    sys.executable,
                    ROOT / "scripts" / "bundle.py",
                    "--python-home", python_home,
                    "--binary", binary,
                    "--output", destination,
                ],
                env=env,
            )
            bundles.append(destination / binary.name)

        # Keep only OS essentials. No Cargo loader variables, Python settings,
        # developer PATH entries, or cache paths can help the packaged apps.
        runtime_env = {
            key: value
            for key, value in os.environ.items()
            if key.upper() in {"SYSTEMROOT", "WINDIR", "TEMP", "TMP", "HOME"}
        }
        runtime_env["PATH"] = (
            str(Path(os.environ["SYSTEMROOT"]) / "System32")
            if os.name == "nt"
            else "/usr/bin:/bin"
        )
        with hidden_directories([cache, target, offline_target]):
            for binary in bundles:
                run([binary], cwd=work, env=runtime_env)
        print(
            f"Python {minor}: no-C-compiler, downstream, offline-cache, and relocated-bundle checks passed.",
            flush=True,
        )


if __name__ == "__main__":
    main()
