# Language bindings

Pick the language that fits your app—the Spotify logic still runs in the same Rust SDK. The shared FFI layer lives in `crates/spotcore-ffi`, with language wrappers, tests and examples under `bindings/`. The layout follows UniLyrics’ separation of core, FFI, wrappers and scripts.

Local lifecycle checks now cover Python, C#, Java, Kotlin, Swift, experimental Dart, C, C++, Go and native Node.js/TypeScript on Linux. See the [binding matrix and setup](../bindings/README.md) for exact toolchains, generation commands and the remaining language blockers.

All wrappers expose typed authentication/account/search/playback/event starter methods plus JSON operation access to all current service domains and playback controls. C/C++/Go use a guarded JSON C interface over the shared worker, and Node uses napi-rs; their implementation does not depend on BoltFFI's WASM or limited C generators. The operation reference records the expanded domain coverage. Generated files and local packages are ignored build artifacts; nothing has been published.

Engine discovery, account checks, browser requirements and Soloist fallback remain inside SpotCore. Language support does not add Android or iOS playback support. Live checks on a Free account now cover Node and Python on Linux: Home, search, library, liked tracks, playlists, devices, friend activity and track details; save/restore membership and temporary playlist creation/editing/removal; and web playback state, seek, volume, quality, queue operations and shutdown. Python also received a typed playback event. These are sampled operation checks, not live verification of all 171 operations. Audible output was not independently confirmed; the other language wrappers and Windows/macOS still need live checks.

## Session configuration

Create a session, then call `login(cookie, options, native_key)` using the naming conventions of your language wrapper. The cookie is the account's `sp_dc`; `native_key` is optional. The shared `SessionOptions` fields are:

| Field | Meaning |
| --- | --- |
| `app_id` | Application storage namespace; letters, digits, `-` and `_ |
| `device_name` | Nonempty Connect name for managed playback |
| `playback` | `false` for browser-free HTTP services; `true` prepares local playback at login |
| `data_dir` | Optional writable storage root; omit to use OS discovery |

Node uses camelCase (`appId`, `deviceName`, `dataDir`); other wrappers follow their generated types. Consult each [language guide](../bindings/README.md) for construction and build commands. A native key is used only when `playback` is true; eligible Premium sessions attempt Soloist, with Web fallback if startup fails. `SessionInfo` returns the account, selected engine and optional fallback reason. This is the selection result, not confirmation of audible output.

The bindings expose session options rather than every Rust builder setter. Custom browser/CDM executable paths use the documented [environment overrides](configuration.md#environment-overrides). Apps retain ownership of credential storage and should explicitly shut down sessions before switching accounts or exiting.

## Service and playback operations

Every language also exposes the complete current service-domain and playback operation inventory through `invoke`. This complements the typed starter methods; its arguments and results use JSON so richer SDK/Spotify fields are preserved. The [operation reference](binding-operations.md) lists every method and argument. This is functional access to those methods, not a promise of fully typed models for every language. SDK restrictions, unsupported methods and account requirements still apply. No mobile playback host is added by a language binding.

For example, check whether a track is saved (after signing in):

### Python

```python
import json
saved = json.loads(await session.invoke(
    "library.contains", json.dumps({"values": [track_uri]})
))
```

### JavaScript / TypeScript

```typescript
const saved = await session.invoke("library.contains", { values: [trackUri] });
const home = await session.invoke("home.feed_info", {
  timezone: "Asia/Colombo", facet: null, items_per_shelf: 10,
});
await session.invoke("playback.set_quality", { quality: "VeryHigh" });
```

Unlike the starter Node DTOs, operation results retain snake_case SDK fields and original Spotify JSON keys. They are typed as `JsonValue`; narrow or validate the specific response in your app.

### Go

```go
raw, err := session.Invoke("library.contains", map[string]any{
    "values": []string{trackURI},
})
// Check err, then json.Unmarshal(raw, &saved) into []bool.
```

### C#

```csharp
var json = await session.Invoke("library.contains",
    JsonSerializer.Serialize(new { values = new[] { trackUri } }));
// System.Text.Json: JsonSerializer.Deserialize<bool[]>(json)
```

### Java

```java
// Encode arguments with your app's JSON library.
String json = session.invoke("library.contains", argumentsJson).join();
```

### Kotlin

```kotlin
val json = session.invoke("library.contains", argumentsJson)
```

### Swift

```swift
let json = try await session.invoke(
    operation: "library.contains", argumentsJson: argumentsJson
)
```

### Dart

```dart
final saved = jsonDecode(await session.invoke('library.contains',
    jsonEncode({'values': [trackUri]})));
```

### C++

```cpp
// Both invoke() and call() return the C interface's {ok,result/error} envelope.
auto response = session.invoke("library.contains", arguments_json);
```

### C

```c
/* Encode using your JSON library, then pass the bytes to spotcore_session_call. */
const char *request = "{\"op\":\"invoke\",\"operation\":\"library.contains\","
                      "\"arguments\":{\"values\":[\"spotify:track:example\"]}}";
char *response = spotcore_session_call(session, (const uint8_t *)request, strlen(request));
/* Inspect ok/result/error, then spotcore_string_free(response). */
```

`operations()` returns the operation/argument inventory without login (JSON text for BoltFFI languages, parsed objects for Go/Node, an envelope for C/C++). C callers send `{"op":"operations"}`. Use JSON serializers for cookies, names, descriptions and other user input; never concatenate those values into JSON. Durations in operation arguments are milliseconds; see the reference for enum and queue-item formats. Existing `next_event`/`poll_event` methods handle events.

Sessions still use the existing cookie/options/native-key login and explicit shutdown. Create separate sessions for separate accounts; apps own credential storage. Builder/process-management utilities and Rust `AccountStore` internals are not exposed by this session-operation interface. Operations run sequentially on the session worker, so a slow service call can delay delivery of player events. No calls are automatically retried after ambiguous cancellation.

For targeted control, `playback.execute_on` takes the SDK's serialized `Command`:

```typescript
await session.invoke("playback.execute_on", {
  device: deviceId, command: { SetVolume: 0.5 },
});
await session.invoke("playback.seek", { position: 30_000 });
await session.invoke("playback.set_shuffle_mode", { mode: "Smart" });
```

`QueueItem` is `{"uri":"spotify:track:...","uid":"..."}`; take the UID from `playback.queue`, rather than inventing one. For device quality methods, pass a complete device object returned by `devices.list`. `Command` uses Rust's enum serialization, including `{ "secs": 30, "nanos": 0 }` for nested durations; only direct duration parameters use integer milliseconds. Rust `u64` values above JavaScript's safe-integer limit cannot be represented exactly as Node numbers. The JSON request cap is 1 MiB, including cover-image byte arrays.

For web playback tests, wait for a playing snapshot whose position advances before seeking. Capability values can be incomplete during startup, and command completion does not mean the next snapshot already reflects the change; observe events or poll for the expected state. This mattered in the live Node/Python checks.

CI now includes offline Node/TypeScript binding checks on Linux, Windows and macOS, plus FFI/Node MSRV checks. The workflow has been prepared locally; remote platform results are not yet available. No live credentials are used in CI.
