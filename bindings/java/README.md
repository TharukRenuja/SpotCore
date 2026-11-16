# Java binding

```sh
python3 scripts/generate_bindings.py java
python3 scripts/check_bindings.py java
```

Requires BoltFFI 0.32.1, Rust, JDK 17+ and Clang. The helper infers `JAVA_HOME` from `javac` when unset, compiles generated JNI glue against the Rust shared library, compiles the generated Java and [search example](examples/SearchExample.java), then runs offline lifecycle checks.

Methods return `CompletableFuture`. `Session` is `AutoCloseable`. Await/join `shutdown()` before `close()`. Async failures expose `BindingError`. A future's `join()` wraps it in `CompletionException`. Blocking joins in the example are for a CLI/background thread, not a UI or native-completion callback thread.

**The JNI helper supports Linux/macOS.** Windows packaging requires separate integration. Build and package the generated Java classes, JNI shim and Rust library together. See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
