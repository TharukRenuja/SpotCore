# C session interface

The C ABI exposes the shared Rust session worker through validated UTF-8 JSON. **Use the public header and native library from the same SDK revision.**

```sh
python3 scripts/check_bindings.py c
```

The local test builds `spotcore-ffi`, links [test.c](test.c), and exercises errors and cleanup without credentials. **Use [spotcore.h](spotcore.h) as the public header and link the target platform's `spotcore_ffi` shared library.** Validate loader paths on your deployment OS.

## Requests and responses

```json
{"op":"login","cookie":"APP_SUPPLIED_COOKIE","options":{"app_id":"my-c-app","device_name":"My Music App","playback":false,"data_dir":null},"native_key":null}
```

Other requests:

```json
{"op":"account"}
{"op":"search_tracks","query":"Miles Davis","limit":10,"offset":0}
{"op":"play","uri":"spotify:track:TRACK_ID"}
{"op":"pause"}
{"op":"poll_event"}
{"op":"shutdown"}
```

Responses are `{"ok":true,"result":...}` or `{"ok":false,"error":{"code":"...","message":"..."}}`. DTO field names use snake_case. `poll_event` returns `null` when no event is queued. It reads the local event buffer without contacting Spotify. `next_event` is rejected in this blocking interface, avoiding an unbounded wait that could prevent wrapper cleanup.

## Ownership and safety

- Create a session with `spotcore_session_new`. Destroy it exactly once with `spotcore_session_free`. The optional creation error is also an owned string.
- Free every returned string exactly once with `spotcore_string_free`. Never use libc `free` on Rust allocations. Null frees are accepted.
- Keep the handle alive until every call finishes. Never free a handle during a call, use a freed pointer or share ownership of the same raw handle.
- Input buffers are borrowed only for the call. Requests are capped at 1 MiB and must be valid UTF-8/JSON. Required fields, integer ranges and operation names are validated. Malformed requests do not echo credentials in errors.
- Calls block while the worker completes them. Run network calls off a GUI thread. Explicit `shutdown` reports cleanup errors. The final handle free waits for shutdown and discards its error. Panic containment requires an unwind-enabled build. The repository's default optimized release profile uses panic abort.

**The ABI is experimental.** Pin the SDK revision and build the header and shared library from the same source. Typed starter calls and the JSON `invoke` interface share the session worker. [Engine requirements](../../docs/engines.md) and native-to-Web fallback apply unchanged. Distribute the public header with the matching native library.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
