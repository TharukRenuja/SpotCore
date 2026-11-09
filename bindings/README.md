# Language binding trials

All bindings call `crates/spotcore-ffi`; Spotify logic and engine selection stay in the Rust SDK. These are local trials, not published packages. Generated code, local binaries and build products are ignored by Git.

| Language | Current result |
| --- | --- |
| [Python](python/README.md) | Generated wheel and offline integration test passed on Linux/Python 3.13 |
| [C# / .NET](dotnet/README.md) | Generated sources, .NET 8 runtime lifecycle test and example compilation passed on Linux |
| [Java](java/README.md) | Generated Java/JNI, JDK 17 lifecycle test and example compilation passed on Linux |
| [Kotlin](kotlin/README.md) | Generated Kotlin/JNI, JVM coroutine lifecycle test and example compilation passed on Linux |
| [Swift](swift/README.md) | Generated Swift/C interface, Swift 6.3 compiler lifecycle test and example compilation passed on Linux |
| [Dart](dart/README.md) | Experimental generator; Dart 3.13 native lifecycle test and static analysis passed on Linux |
| [C](c/README.md) | Handwritten guarded ABI over the shared worker; input/lifecycle checks passed on Linux |
| [C++](cpp/README.md) | Move-only RAII wrapper, JSON responses and cleanup checks passed on Linux |
| [Go](go/README.md) | Typed cgo wrapper; lifecycle/concurrency tests passed with the race detector on Linux |
| [Node.js / TypeScript](node/README.md) | Native napi-rs addon; Promises, structured errors and TypeScript checks passed on Linux |
| Ruby | Strict BoltFFI generation still rejects the session class and records; no wrapper provided |

Every working target exposes the same small surface: login/account details, track search with basic typed media and pagination, play/pause, playback events, and shutdown (C/C++/Go use non-blocking event polling). C++ keeps JSON envelopes; Go and Node expose typed results. The shared `invoke` interface additionally exposes every current service-domain method and playback control. See the [operation reference](../docs/binding-operations.md) and [language examples](../docs/bindings.md#service-and-playback-operations). These broader results use JSON, not per-language typed DTOs. Tests cover async errors, configuration marshalling and worker lifetime; positive account/search/ audio operations still need broader live verification per language. Sampled Free-account service/mutation and web-player-state checks passed for Node and Python on Linux; see the [verification scope](../docs/bindings.md). Audible output and the remaining languages/platforms are not established by those samples.

```sh
cargo install boltffi_cli --version 0.32.1 --locked
python3 scripts/generate_bindings.py dotnet
python3 scripts/check_bindings.py dotnet
```

Replace `dotnet` with `java`, `kotlin`, `swift` or `dart`. `generate_bindings.py all` generates the six BoltFFI targets, including the local Python wheel, and therefore needs a Python interpreter with development headers. Every generation uses `--deny-skipped`; unsupported declarations fail instead of silently disappearing. Dart is explicitly experimental. C/C++/Go do not use BoltFFI's async-incompatible C generator; Node uses napi-rs rather than the WASM target. For these targets run `scripts/check_bindings.py c`, `cpp`, `go` or `node`; Node can also be built with `scripts/build_node.py`. See each guide for compiler/runtime prerequisites.

## Engines and platforms

- `playback=false` means HTTP services: no browser, launcher or Soloist required.
- `playback=true` enables managed local playback. Web audio still needs the launcher, compatible browser/Qt helper, Widevine and AAC. A freshly verified Premium account plus an opted-in native key can use Soloist; managed native startup retains the known fallback limitation.
- Generated language libraries do not bundle browser runtimes, DRM or Soloist. The SDK's existing discovery and downloader do that work; see [engine setup](../docs/engines.md).
- A Kotlin or Swift wrapper does not establish Android or iOS playback support. Windows/macOS language runtime tests and mobile packaging are not verified here.
- Call the async SDK shutdown before releasing the wrapper handle or closing the app. Cancelling an await does not undo a queued operation. Do not retry an ambiguous cancelled mutation automatically.

The local JNI test helper supports Linux/macOS; only Linux has been exercised. The Swift test helper compiles generated source with a C module map and the Rust shared library; it does not build an Apple XCFramework. No registry upload or release workflow is involved.
