# Kotlin/JVM trial

```sh
python3 scripts/generate_bindings.py kotlin
python3 scripts/check_bindings.py kotlin
```

Requires BoltFFI 0.32.1, Rust, Clang, a JDK and `kotlinc` with the `kotlinx-coroutines-core-jvm.jar` dependency. The test helper finds that JAR in the compiler distribution, or accepts `--coroutines /path/to/the.jar`. Kotlin 2.4/JDK 17 offline lifecycle checks and the [search example](examples/SearchExample.kt) compiled and passed on Linux.

The generated API uses suspend functions, nullable option values and `BindingError` exceptions. Await `shutdown()` before `close()`/leaving `use`. The generated desktop loader finds the local JNI shim via `java.library.path`; the JNI shim links the Rust shared library. `generated/` must be rebuilt from the common FFI source, not edited by hand.

This is desktop JVM coverage. The Android-named BoltFFI configuration also controls Kotlin generation; it does not mean Android playback is supported. See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields; existing permissions and backend limitations apply.
