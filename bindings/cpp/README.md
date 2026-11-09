# C++ session wrapper

```sh
python3 scripts/check_bindings.py cpp
```

[spotcore.hpp](spotcore.hpp) is a C++17 header over the [C ABI](../c/README.md). It owns the session and response buffers, rejects copies, supports moves and serializes calls/close. Destruction waits for worker shutdown. Keep the wrapper object alive while its methods run; moving or destroying the object concurrently with a call is not supported.

```cpp
#include "spotcore.hpp"
spotcore::session spotify;
std::string account_json = spotify.account();
std::string event_json = spotify.poll_event();
```

SDK successes and errors remain JSON envelopes, as in UniLyrics' C++ interface, so applications can use their existing JSON library. Factory/ownership failures throw C++ exceptions. `call(json)` exposes login, search and play as described in the C protocol. Build JSON with a serializer; do not interpolate cookies into unescaped JSON text.

The [CLI example](examples/client.cpp) reads newline-delimited requests from stdin without echoing credential input. Use `poll_event` for local updates; this interface deliberately rejects the asynchronous `next_event` operation. Network calls and destructor cleanup can block, so keep them off the UI thread.

Only local Linux compilation/lifecycle checks have passed. Positive live calls, non-Linux linking and full SDK domain coverage remain unverified. Browser/DRM and Soloist requirements are unchanged. Nothing is packaged or published.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields; existing permissions and backend limitations apply.
