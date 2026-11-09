# Swift trial

```sh
python3 scripts/generate_bindings.py swift
python3 scripts/check_bindings.py swift
```

Requires BoltFFI 0.32.1, Rust and a Swift compiler. The helper uses `ffi/module.modulemap` to import the generated C header and links the Rust shared library. Offline lifecycle checks and the [search example](examples/SearchExample.swift) passed with the Swift 6.3 compiler on Linux, using its default language mode.

Use throwing `Session()` construction and async throwing methods. Errors are `BindingError` values with `code` and `message`. Await `shutdown()` before releasing the session. No Apple framework or Swift package is published or bundled here; macOS/iOS packaging and runtime behavior remain unverified.

See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields; existing permissions and backend limitations apply.
