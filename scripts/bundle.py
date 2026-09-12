#!/usr/bin/env python3
"""Package a native pybundle application and its matching Python distribution.

Run on the application's target OS. Python is needed only to run this helper,
not on the machine running the resulting application. No third-party modules.
"""
import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile


def bundle(binary: Path, home: Path, output: Path) -> None:
    binary = binary.resolve(strict=True)
    home = home.resolve(strict=True)
    output = output.resolve(strict=False)
    if output.exists():
        raise ValueError(f"output already exists: {output}; choose a new directory")
    if binary.name in {"python", "bin"} or any(c in binary.name for c in "\n\r"):
        raise ValueError("binary name cannot be 'python', 'bin', or contain newlines")
    if not binary.is_file() or not (home / ".pybundle-version").is_file():
        raise ValueError("provide an application binary and a Python home prepared by pybundle")
    if home in output.parents:
        raise ValueError("output cannot be inside the source Python distribution")
    version = (home / ".pybundle-version").read_text().strip()
    minor = ".".join(version.split(".")[:2])
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".pybundle-", dir=output.parent) as temporary:
        stage = Path(temporary) / "bundle"
        stage.mkdir()
        # Preserve upstream libraries, stdlib, resources and licenses together.
        # Windows distribution does not require symlink creation privileges.
        shutil.copytree(home, stage / "python", symlinks=os.name != "nt")
        if sys.platform == "linux":
            (stage / "bin").mkdir()
            shutil.copy2(binary, stage / "bin" / binary.name)
            launcher = stage / binary.name
            launcher.write_text(
                '#!/bin/sh\n'
                'set -eu\n'
                'bundle=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)\n'
                'export PYBUNDLE_PYTHON_HOME="$bundle/python"\n'
                'export LD_LIBRARY_PATH="$bundle/python/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"\n'
                'exec "$bundle/bin/"' + shlex.quote(binary.name) + ' "$@"\n'
            )
            launcher.chmod(0o755)
        elif sys.platform == "darwin":
            app = stage / binary.name
            shutil.copy2(binary, app)
            library = f"libpython{minor}.dylib"
            subprocess.run(
                ["install_name_tool", "-change", f"@rpath/{library}",
                 f"@executable_path/python/lib/{library}", str(app)], check=True
            )
            # Editing load commands invalidates the Rust binary's ad-hoc signature.
            # A distributor replaces this with their signing identity before shipping.
            subprocess.run(["codesign", "--force", "--sign", "-", str(app)], check=True)
        elif os.name == "nt":
            shutil.copy2(binary, stage / binary.name)
            # The Windows loader must find these before Rust's main() can run.
            for library in home.glob("*.dll"):
                shutil.copy2(library, stage / library.name)
        else:
            raise ValueError(f"unsupported packaging host: {sys.platform}")
        stage.rename(output)
    print(output / binary.name)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--python-home", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        bundle(args.binary, args.python_home, args.output)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"pybundle packaging failed: {error}\n")


if __name__ == "__main__":
    main()
