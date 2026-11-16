# Python binding

Build a Python wheel from the shared Rust SDK using BoltFFI. See [binding setup](../README.md) for other languages. **The wheel must match your OS, architecture and Python ABI.**

## Build locally

Use Rust 1.89+, a C compiler, and Python 3.10+ with pip and development headers. Install the matching generator yourself, or point `BOLTFFI_BIN` at its executable:

```sh
cargo install boltffi_cli --version 0.32.1 --locked
python3 scripts/generate_bindings.py --python python3
```

The script runs from the repository root, generates Python/C bridge sources under `bindings/python/generated`, and builds a local wheel under `bindings/python/generated/wheelhouse`. It never uploads anything. Generated files and wheels are ignored by Git. Edit `crates/spotcore-ffi/src`, then regenerate.

Install the wheel in an isolated environment using the same Python interpreter:

```sh
python3 -m venv bindings/python/.venv
bindings/python/.venv/bin/python -m pip install bindings/python/generated/wheelhouse/*.whl
bindings/python/.venv/bin/python -m unittest discover -s bindings/python/tests -v
```

On Windows use `.venv\Scripts\python.exe`. The offline tests need no Spotify credentials, browser, DRM or Soloist. A local wheel is specific to its host and Python ABI. It is not a universal cross-platform package.

## Use the SDK

```python
from spotcore import Session, SessionOptions

spotify = Session()
try:
    info = await spotify.login(sp_dc, SessionOptions(
        app_id="my-music-app", device_name="My Music App",
        playback=False, data_dir=None,
    ), None)
    tracks = await spotify.search_tracks("Miles Davis", 10, 0)
    for track in tracks.items:
        print(track.name, track.uri)
finally:
    await spotify.shutdown()
```

Use this inside an async function. The app supplies `sp_dc`. Do not print it. `playback=False` initializes HTTP services without a browser. For local audio, set `playback=True` and optionally supply the native key as the third `login` argument. `SessionInfo.engine` is `services`, `web` or `native`, and `fallback_reason` reports native fallback.

```python
await spotify.play(track_uri)
event = await spotify.next_event()
await spotify.pause()
```

Web playback needs the existing launcher, browser engine, Widevine and AAC setup. The wheel contains the SDK shared library, not Qt, a browser or Soloist. The optional native runtime is downloaded by SpotCore. See [engine setup](../../docs/engines.md). For runnable examples see [search](../../examples/python/search.py) and [playback](../../examples/python/play.py).

## Session lifetime and limits

- One worker thread owns the Rust session and runtime. Calls execute in order. Call `shutdown()` in a `finally` block. Dropping the Python object also requests cleanup, but interpreter teardown is not a substitute for explicit shutdown.
- One login per `Session`. Use a new object for another account. Saved-account management is owned by your application.
- `BindingErrorException.error` contains a stable `code` and a readable `message`. Signed-out calls report `not_authenticated`. Calls after shutdown report `closed`.
- Search returns basic typed media data and pagination, not the full catalog model.
- `next_event()` is for one event consumer in local playback mode. Services mode reports `unsupported`. Events buffer up to 64 entries. Slow readers can receive `events_lagged`. Queue/capability changes are notifications, not complete payloads.
- Cancelling a Python await stops waiting. A command already sent to the worker can still execute. Never automatically retry a cancelled mutation.
- Managed native startup can fall back to Web. Inspect the selected engine and fallback reason. This binding does not change account entitlements or engine compatibility.

**Verify account access and audio on your target runtime.** Offline tests cover wrapper behaviour without Spotify credentials. SpotCore is GPL-3.0-only. See the root license and third-party notices.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
