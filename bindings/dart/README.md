# Experimental Dart trial

```sh
python3 scripts/generate_bindings.py dart
python3 scripts/check_bindings.py dart
```

Requires BoltFFI 0.32.1, Rust and Dart 3.10.8+. This passed with Dart 3.13 on Linux. The helper copies the host Rust library into the generated package's native assets, resolves this local workspace's Dart dependencies and runs static analysis plus offline lifecycle checks. The [search example](examples/search.dart) uses a private cookie file and can be run from this directory after the checks:

```sh
SPOTCORE_COOKIE_FILE=/absolute/path/to/private-cookie dart run examples/search.dart
```

Import `package:spotcore_ffi/spotcore_ffi.dart`. The Dart-only BoltFFI overlay matches the package name to the Rust artifact name; otherwise BoltFFI 0.32.1's native-asset hook registers a different asset ID from the generated Dart library. No generated source patch is needed.

Await `shutdown()` before `dispose$()`. A CLI must also call `shutdownBoltffi()` once all sessions and native callbacks have finished, so Dart can close its process-lifetime callback listeners. Do not call that global function while any session or pending native operation is still in use.

Dart support is experimental upstream. Flutter, mobile playback and Windows/macOS runtime behavior have not been verified. Nothing is uploaded to pub.dev. See [shared scope and engine requirements](../README.md).

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields; existing permissions and backend limitations apply.
