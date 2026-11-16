# Kotlin/JVM binding

```sh
python3 scripts/generate_bindings.py kotlin
python3 scripts/check_bindings.py kotlin
```

Requires BoltFFI 0.32.1, Rust, Clang, a JDK and `kotlinc` with the `kotlinx-coroutines-core-jvm.jar` dependency. The test helper finds that JAR in the compiler distribution, or accepts `--coroutines /path/to/the.jar`. The helper checks the lifecycle and compiles the [search example](examples/SearchExample.kt).

The generated API uses suspend functions, nullable option values and `BindingError` exceptions. Await `shutdown()` before `close()`/leaving `use`. The generated desktop loader finds the local JNI shim via `java.library.path`. The JNI shim links the Rust shared library. `generated/` must be rebuilt from the common FFI source, not edited by hand.

**This guide targets desktop JVM applications.** The Android-named BoltFFI configuration controls Kotlin generation. It does not supply an Android playback host. See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
