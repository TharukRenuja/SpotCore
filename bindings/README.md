# Language bindings

All bindings use `crates/spotcore-ffi`. **Spotify services, playback routing and engine selection run in the Rust SDK.** Build the wrapper and native library for each deployment target.

| Language | Integration | Required tools |
| --- | --- | --- |
| [Python](python/README.md) | Generated wheel | Rust, C compiler, Python 3.10+ with development headers, BoltFFI 0.32.1 |
| [C# / .NET](dotnet/README.md) | Generated C# sources | Rust, .NET SDK, BoltFFI 0.32.1 |
| [Java](java/README.md) | Generated Java/JNI | Rust, JDK 17+, Clang, BoltFFI 0.32.1 |
| [Kotlin](kotlin/README.md) | Generated Kotlin/JNI | Rust, JDK, Clang, Kotlin and coroutines, BoltFFI 0.32.1 |
| [Swift](swift/README.md) | Generated Swift/C interface | Rust, Swift compiler, BoltFFI 0.32.1 |
| [Dart](dart/README.md) | Experimental generated native assets | Rust, Dart 3.10.8+, BoltFFI 0.32.1 |
| [C](c/README.md) | Guarded JSON C ABI | Rust and a C compiler |
| [C++](cpp/README.md) | Move-only C++ wrapper | Rust and a C++17 compiler |
| [Go](go/README.md) | Typed cgo wrapper | Rust, Go with cgo enabled, C compiler |
| [Node.js / TypeScript](node/README.md) | Native napi-rs addon | Rust, Node.js 18+, TypeScript for declaration checks |

Ruby is not supported by the current generator. Dart remains experimental.

**Typed starter methods** cover login, account details, track search, play/pause, events and shutdown. C, C++ and Go use non-blocking event polling. C++ keeps JSON envelopes, while Go and Node provide typed starter results. **`invoke()` provides the full service and playback operation inventory** with JSON arguments and results. See the [operation reference](../docs/binding-operations.md) and [language examples](../docs/bindings.md#service-and-playback-operations).

## Generate and check a binding

Run from the repository root:

```sh
cargo install boltffi_cli --version 0.32.1 --locked
python3 scripts/generate_bindings.py dotnet
python3 scripts/check_bindings.py dotnet
```

Replace `dotnet` with `java`, `kotlin`, `swift` or `dart`. `generate_bindings.py all` generates the six BoltFFI targets, including the local Python wheel, and therefore needs a Python interpreter with development headers. Every generation uses `--deny-skipped`. Unsupported declarations fail instead of silently disappearing. Dart is explicitly experimental. C/C++/Go do not use BoltFFI's async-incompatible C generator. Node uses napi-rs rather than the WASM target. For these targets run `scripts/check_bindings.py c`, `cpp`, `go` or `node`. Node can also be built with `scripts/build_node.py`. See each guide for compiler/runtime prerequisites.

## Engines and platforms

- `playback=false` means HTTP services: no browser, launcher or Soloist required.
- `playback=true` enables managed local playback. Web audio still needs the launcher, compatible browser/Qt helper, Widevine and AAC. A freshly verified Premium account plus an opted-in native key can use Soloist. Inspect the selected engine and fallback reason when native setup fails.
- Generated language libraries do not bundle browser runtimes, DRM or Soloist. Configure the Web runtime separately. The SDK can download the opted-in Soloist runtime. See [engine setup](../docs/engines.md).
- **A language wrapper does not supply a mobile playback host.** Validate deployment packaging and runtime behaviour for each OS.
- **Call async SDK shutdown before releasing the wrapper or closing the app.** Cancelling an await does not undo a queued operation. Do not retry an ambiguous cancelled mutation automatically.

The JNI helper supports Linux/macOS. The Swift helper compiles generated source with a C module map and the Rust shared library. It does not build an Apple XCFramework. Package the resulting native library with your app and configure its loader path.
