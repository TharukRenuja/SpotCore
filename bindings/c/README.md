# C session interface

This is a handwritten C ABI over the shared Rust worker, not BoltFFI's sync-only C generator. It exposes the same trial operations through validated UTF-8 JSON.

```sh
python3 scripts/check_bindings.py c
```

The local test builds `spotcore-ffi`, links [test.c](test.c), and exercises errors and cleanup without credentials. Linux has been verified; Windows/macOS runtime linking remains unverified. Use [spotcore.h](spotcore.h) as the public header and link the platform's `spotcore_ffi` shared library.

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

Responses are `{"ok":true,"result":...}` or `{"ok":false,"error":{"code":"...","message":"..."}}`. DTO field names use snake_case. `poll_event` returns `null` when no event is queued; it reads the local event buffer without contacting Spotify. `next_event` is rejected in this blocking interface, avoiding an unbounded wait that could prevent wrapper cleanup.

## Ownership and safety

- Create a session with `spotcore_session_new`; destroy it exactly once with `spotcore_session_free`. The optional creation error is also an owned string.
- Free every returned string exactly once with `spotcore_string_free`. Never use libc `free` on Rust allocations. Null frees are accepted.
- Keep the handle alive until every call finishes. Never free a handle during a call, use a freed pointer or share ownership of the same raw handle.
- Input buffers are borrowed only for the call. Requests are capped at 1 MiB and must be valid UTF-8/JSON. Required fields, integer ranges and operation names are validated; malformed requests do not echo credentials in errors.
- Calls block while the worker completes them. Run network calls off a GUI thread. Explicit `shutdown` reports cleanup errors; the final handle free waits for shutdown and discards its error. Panic containment requires an unwind-enabled build; the repository's default optimized release profile uses panic abort.

The ABI is experimental, not a stable system-library contract. It does not expose every Rust domain yet. [Engine requirements](../../docs/engines.md) and the known managed-native fallback apply unchanged. No C library package is published.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields; existing permissions and backend limitations apply.
