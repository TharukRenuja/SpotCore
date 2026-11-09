#!/usr/bin/env python3
"""Compile and run offline lifecycle tests against the generated bindings."""
import argparse
import os
from pathlib import Path
import shutil
import shlex
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def run(command, cwd=ROOT, **kwargs):
    subprocess.run([str(arg) for arg in command], check=True, cwd=cwd, timeout=180, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("language", choices=["c", "cpp", "go", "node", "dotnet", "java", "kotlin", "swift", "dart"])
    parser.add_argument("--coroutines", type=Path, help="Kotlin coroutines JAR; defaults to the compiler distribution")
    args = parser.parse_args()
    run([sys.executable, ROOT / "scripts" / "generate_ffi_operations.py", "--check"])
    folder = ROOT / "bindings" / args.language
    generated = folder / "generated"
    if args.language not in ("c", "cpp", "go", "node") and not generated.exists():
        parser.error("Generate this language first with scripts/generate_bindings.py")
    build = folder / "build"
    build.mkdir(exist_ok=True)
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target")).resolve() / "debug"
    if args.language == "node":
        run([sys.executable, ROOT / "scripts" / "build_node.py"])
        run(["node", "--test", "tests/lifecycle.test.cjs"], cwd=folder)
        compiler = shutil.which("tsc")
        if not compiler:
            parser.error("A TypeScript compiler (tsc) is required")
        run([compiler, "-p", "tests/tsconfig.json"], cwd=folder)
        return
    run(["cargo", "build", "-p", "spotcore-ffi", "--locked"])
    env = os.environ.copy()
    path_key = "PATH" if sys.platform == "win32" else "DYLD_LIBRARY_PATH" if sys.platform == "darwin" else "LD_LIBRARY_PATH"
    env[path_key] = str(target) + os.pathsep + env.get(path_key, "")
    if args.language in ("c", "cpp", "go"):
        if sys.platform not in ("linux", "darwin"):
            parser.error("The local C/C++/cgo test helper supports Linux/macOS; Windows linking is not verified")
        if args.language == "go":
            env["CGO_ENABLED"] = "1"
            env["CGO_LDFLAGS"] = env.get("CGO_LDFLAGS", "") + " " + shlex.quote("-L" + str(target))
            run(["go", "test", "-race", "./..."], cwd=folder, env=env)
        else:
            cpp = args.language == "cpp"
            executable = build / "lifecycle"
            run(["clang++" if cpp else "clang", "-std=c++17" if cpp else "-std=c11", "-Wall", "-Wextra", "-Werror",
                 *(["-pthread"] if cpp else []), folder / ("test.cpp" if cpp else "test.c"), "-L" + str(target), "-lspotcore_ffi",
                 "-Wl,-rpath," + str(target), "-o", executable])
            run([executable], env=env)
        print(f"{args.language} lifecycle checks passed")
        return
    if args.language == "dart":
        package = generated / "spotcore_ffi"
        host = subprocess.check_output(["rustc", "-vV"], text=True)
        triple = next(line.split(": ", 1)[1] for line in host.splitlines() if line.startswith("host: "))
        name = "spotcore_ffi.dll" if sys.platform == "win32" else "libspotcore_ffi.dylib" if sys.platform == "darwin" else "libspotcore_ffi.so"
        native = package / "native" / triple
        native.mkdir(parents=True, exist_ok=True)
        shutil.copy2(target / name, native / name)
        run(["dart", "pub", "get"], cwd=folder)
        run(["dart", "analyze", "tests", "examples"], cwd=folder)
        run(["dart", "run", "tests/lifecycle_test.dart"], cwd=folder, env=env)
        return
    if args.language == "dotnet":
        run(["dotnet", "run", "--project", folder / "Tests" / "Tests.csproj"], env=env)
        return
    if args.language in ("java", "kotlin"):
        if sys.platform not in ("linux", "darwin"):
            parser.error("The local JNI test helper supports Linux/macOS; Windows JNI packaging is not verified")
        javac = shutil.which("javac")
        if not javac:
            parser.error("A JDK with javac is required")
        java_home = Path(os.environ.get("JAVA_HOME", Path(javac).resolve().parent.parent))
        suffix = "dylib" if sys.platform == "darwin" else "so"
        platform_include = "darwin" if sys.platform == "darwin" else "linux"
        link_mode = "-dynamiclib" if sys.platform == "darwin" else "-shared"
        run(["clang", "-std=c11", link_mode, "-fPIC", "-I" + str(java_home / "include"),
             "-I" + str(java_home / "include" / platform_include), generated / "jni" / "jni_glue.c",
             "-L" + str(target), "-lspotcore_ffi", "-Wl,-rpath," + str(target),
             "-o", build / f"libspotcore_ffi_jni.{suffix}"])
        if args.language == "java":
            classes = build / "classes"
            classes.mkdir(exist_ok=True)
            run([javac, "--release", "17", "-d", classes, *generated.rglob("*.java"), *folder.glob("examples/*.java"), folder / "tests" / "LifecycleTest.java"])
            run(["java", "-Djava.library.path=" + str(build), "-cp", classes, "LifecycleTest"], env=env)
        else:
            compiler = shutil.which("kotlinc")
            if not compiler:
                parser.error("kotlinc is required")
            jar = args.coroutines or Path(compiler).resolve().parent.parent / "lib" / "kotlinx-coroutines-core-jvm.jar"
            if not jar.exists():
                parser.error("Provide --coroutines /path/to/kotlinx-coroutines-core-jvm.jar")
            output = build / "tests.jar"
            run([compiler, *generated.rglob("*.kt"), *folder.glob("examples/*.kt"), folder / "tests" / "LifecycleTest.kt", "-cp", jar, "-include-runtime", "-d", output])
            run(["java", "-Djava.library.path=" + str(build), "-cp", str(output) + os.pathsep + str(jar), "LifecycleTestKt"], env=env)
        return
    run(["swiftc", "-parse-as-library", "-I", folder / "ffi", "-L", target, "-lspotcore_ffi",
         "-Xlinker", "-rpath", "-Xlinker", target, *generated.glob("*.swift"), *folder.glob("examples/*.swift"), folder / "tests" / "LifecycleTest.swift",
         "-o", build / "lifecycle"])
    run([build / "lifecycle"], env=env)


if __name__ == "__main__":
    main()
