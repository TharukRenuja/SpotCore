#!/usr/bin/env python3
"""Generate local language bindings (and Python wheels); never publish them."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
VERSION = "0.32.1"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", nargs="?", default="python", choices=["python", "dotnet", "java", "kotlin", "swift", "dart", "all"])
    parser.add_argument("--python", default=sys.executable, help="Python interpreter with pip and development headers")
    args = parser.parse_args()
    cli = os.environ.get("BOLTFFI_BIN", "boltffi")
    try:
        version = subprocess.check_output([cli, "--version"], text=True).strip()
    except FileNotFoundError:
        parser.error(f"Install boltffi_cli {VERSION}, or set BOLTFFI_BIN")
    if version != f"boltffi {VERSION}":
        parser.error(f"Expected boltffi {VERSION}; found {version}")
    cwd = ROOT / "crates" / "spotcore-ffi"
    languages = ["python", "dotnet", "java", "kotlin", "swift", "dart"] if args.language == "all" else [args.language]
    for language in languages:
        target = "csharp" if language == "dotnet" else language
        command = [cli]
        if language == "dart":
            command.extend(["--overlay", "boltffi.dart.toml"])
        command.extend(["generate", target, "--deny-skipped", "--cargo-arg=--locked"])
        if language == "dart":
            command.append("--experimental")
        subprocess.run(command, cwd=cwd, check=True)
        output = ROOT / "bindings" / language / "generated"
        license_root = output / "spotcore_ffi" if language == "dart" else output
        shutil.copy2(ROOT / "LICENSE", license_root / "LICENSE")
        shutil.copy2(ROOT / "THIRD-PARTY-NOTICES.md", license_root / "NOTICE.md")
        if language == "python":
            subprocess.run([args.python, "-c", "import pathlib,sysconfig; assert (pathlib.Path(sysconfig.get_path('include'))/'Python.h').exists(), 'Python development headers are required'"], check=True)
            subprocess.run([cli, "pack", "python", "--regenerate=false", "--deny-skipped", "--cargo-arg=--locked", "--python", args.python], cwd=cwd, check=True)
            print(f"Local wheels: {output / 'wheelhouse'}")
        else:
            print(f"Generated sources: {output}")



if __name__ == "__main__":
    main()
