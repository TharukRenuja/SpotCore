#!/usr/bin/env python3
"""Build the local Node addon without npm packaging or publishing."""
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
subprocess.run(["cargo", "build", "-p", "spotcore-node", "--locked"], cwd=ROOT, check=True)
target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "debug"
name = "spotcore_node.dll" if sys.platform == "win32" else "libspotcore_node.dylib" if sys.platform == "darwin" else "libspotcore_node.so"
output = ROOT / "bindings" / "node" / "generated"
output.mkdir(exist_ok=True)
shutil.copy2(target / name, output / "spotcore.node")
for file in ["LICENSE", "THIRD-PARTY-NOTICES.md"]:
    shutil.copy2(ROOT / file, output / file)
print(output / "spotcore.node")
