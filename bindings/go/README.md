# Go / cgo wrapper

```sh
python3 scripts/check_bindings.py go
```

Requires Rust, Go with cgo enabled and a C compiler. The helper links the shared Rust library and runs `go test -race ./...`, including example compilation. The module has no third-party Go dependencies. No Go package has been published.

```go
spotify, err := spotcore.NewSession()
if err != nil { return err }
defer spotify.Close()
_, err = spotify.Login(cookie, spotcore.SessionOptions{
    AppID: "my-go-app", DeviceName: "My Music App", Playback: false,
}, nil)
if err != nil { return err }
page, err := spotify.SearchTracks("Miles Davis", 10, 0)
```

See the [read-only example](examples/search/main.go). To build it outside the test helper, provide the Rust library directory through `CGO_LDFLAGS` and the OS dynamic loader path, as well as `-lspotcore_ffi` from the wrapper. Distribution still needs an architecture/OS-matched native library. Cgo does not make cross-compilation automatic.

`Session` must not be copied. Its mutex serializes calls and `Close`. Explicit `Close` is required and idempotent. There is no finalizer guarantee. Calls block while native work completes, so use an application-owned goroutine when needed. The wrapper does not offer cancellation that would falsely imply a queued write was undone.

`Error` exposes `Code` and `Message`. Results use typed Go structs. `PollEvent` returns a nil event when the local buffer is empty and an error in services-only mode. It does not poll Spotify over the network. A native key is the optional third `Login` argument. Normal engine discovery, account rules and native fallback remain inside SpotCore.

**Verify linking, lifecycle and account/audio access on each target OS.** Use `invoke()` for domain operations beyond the typed starter methods.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
