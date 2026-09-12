#!/usr/bin/env python3
"""Build real consumers, then run their relocated bundles without build caches.

Python is only the test/packaging driver. PyO3 interpreter discovery is poisoned,
and the packaged applications run with a separate, minimal environment.
"""

import contextlib
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

        # Running from the fixture loads its checked-in .cargo configuration.
        run(["cargo", "run", "--locked"], cwd=PYO3_FIXTURE, env=env)
        run(["cargo", "run", "--locked", "--release"], cwd=PYO3_FIXTURE, env=env)

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
            # Spaces also exercise launcher quoting and home-path encoding.
            destination = work / f"relocated bundle {binary.stem}"
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
        print("All downstream, offline-cache, and relocated-bundle checks passed.", flush=True)


if __name__ == "__main__":
    main()
