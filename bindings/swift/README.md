# Swift binding

```sh
python3 scripts/generate_bindings.py swift
python3 scripts/check_bindings.py swift
```

Requires BoltFFI 0.32.1, Rust and a Swift compiler. The helper uses `ffi/module.modulemap` to import the generated C header and links the Rust shared library. The helper checks lifecycle and compiles the [search example](examples/SearchExample.swift).

Use throwing `Session()` construction and async throwing methods. Errors are `BindingError` values with `code` and `message`. Await `shutdown()` before releasing the session. **Build and package the native library for your deployment target.** The helper does not create an Apple framework or supply an iOS playback host.

See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
