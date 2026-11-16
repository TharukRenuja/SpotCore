# C# / .NET binding

```sh
python3 scripts/generate_bindings.py dotnet
python3 scripts/check_bindings.py dotnet
```

Requires BoltFFI 0.32.1, Rust and a .NET SDK. The check project targets .NET 8 with the latest installed C# compiler. It compiles `generated/*.cs` and the [search example](examples/SearchExample.cs) with unsafe code enabled, and loads `target/debug/libspotcore_ffi.so` (or the platform's corresponding library).

Use `Session` with async `Login`, `Account`, `SearchTracks`, `Play`, `Pause`, `NextEvent` and `Shutdown`. Read `BindingErrorException.Error.Code` for structured errors. Await `Shutdown()` before disposing the session. Build the generated sources into your application or package them with the matching native library.

See [shared scope and engine requirements](../README.md). **Package the native library for each target OS and verify loading and playback there.**

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
