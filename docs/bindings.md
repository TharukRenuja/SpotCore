# Language bindings

Use SpotCore from Python, JavaScript/TypeScript, C#, Java, Kotlin, Swift, Dart, C, C++ or Go. **Every binding uses the same Rust SDK** for Spotify services, playback, engine discovery and account checks. The shared interface is in `crates/spotcore-ffi`. Build commands and examples are under `bindings/`.

**Build the native library for your deployment OS and architecture.** See the [binding setup](../bindings/README.md) for toolchains and generation commands. Dart support is experimental. Mobile packaging and playback require a compatible host and are not supplied by a language wrapper.

**Typed starter methods** cover authentication, account details, track search, playback and events. **`invoke()` exposes all current service-domain and playback operations** using JSON arguments and results. C, C++ and Go use the guarded C interface. Node uses a native napi-rs addon. Generate the other language interfaces with the pinned BoltFFI version in the build guides.

**Playback requirements are unchanged across languages.** Services-only sessions need credentials but no browser. Local Web audio needs the launcher or embedded supervisor, a compatible browser engine, Widevine and AAC. Native playback needs an eligible Premium account and an opted-in Soloist key. See [engine setup](engines.md) and [playback](playback.md).

## Session configuration

Create a session, then call `login(cookie, options, native_key)` using the naming conventions of your language wrapper. The cookie is the account's `sp_dc`. `native_key` is optional. The shared `SessionOptions` fields are:

| Field | Meaning |
| --- | --- |
| `app_id` | Application storage namespace. Letters, digits, `-` and `_ |
| `device_name` | Nonempty Connect name for managed playback |
| `playback` | `false` for browser-free HTTP services. `true` prepares local playback at login |
| `data_dir` | Optional writable storage root. Omit to use OS discovery |

Node uses camelCase (`appId`, `deviceName`, `dataDir`). Other wrappers follow their generated types. Consult each [language guide](../bindings/README.md) for construction and build commands. A native key is used only when `playback` is true. Eligible Premium sessions attempt Soloist, with Web fallback if startup fails. `SessionInfo` returns the account, selected engine and optional fallback reason. This is the selection result, not confirmation of audible output.

The bindings expose session options rather than every Rust builder setter. Custom browser/CDM executable paths use the documented [environment overrides](configuration.md#environment-overrides). Apps retain ownership of credential storage and should explicitly shut down sessions before switching accounts or exiting.

## Service and playback operations

Every language also exposes the complete current service-domain and playback operation inventory through `invoke`. This complements the typed starter methods. Its arguments and results use JSON so richer SDK/Spotify fields are preserved. The [operation reference](binding-operations.md) lists every method and argument. This is functional access to those methods, not a promise of fully typed models for every language. SDK restrictions, unsupported methods and account requirements still apply. No mobile playback host is added by a language binding.

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

Unlike the starter Node DTOs, operation results retain snake_case SDK fields and original Spotify JSON keys. They are typed as `JsonValue`. Narrow or validate the specific response in your app.

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

`operations()` returns the operation/argument inventory without login (JSON text for BoltFFI languages, parsed objects for Go/Node, an envelope for C/C++). C callers send `{"op":"operations"}`. Use JSON serializers for cookies, names, descriptions and other user input. Never concatenate those values into JSON. Durations in operation arguments are milliseconds. See the reference for enum and queue-item formats. Existing `next_event`/`poll_event` methods handle events.

Sessions still use the existing cookie/options/native-key login and explicit shutdown. Create separate sessions for separate accounts. Apps own credential storage. Builder/process-management utilities and Rust `AccountStore` internals are not exposed by this session-operation interface. Operations run sequentially on the session worker, so a slow service call can delay delivery of player events. No calls are automatically retried after ambiguous cancellation.

For targeted control, `playback.execute_on` takes the SDK's serialized `Command`:

```typescript
await session.invoke("playback.execute_on", {
  device: deviceId, command: { SetVolume: 0.5 },
});
await session.invoke("playback.seek", { position: 30_000 });
await session.invoke("playback.set_shuffle_mode", { mode: "Smart" });
```

`QueueItem` is `{"uri":"spotify:track:...","uid":"..."}`. Take the UID from `playback.queue`, rather than inventing one. For device quality methods, pass a complete device object returned by `devices.list`. `Command` uses Rust's enum serialization, including `{ "secs": 30, "nanos": 0 }` for nested durations. Only direct duration parameters use integer milliseconds. Rust `u64` values above JavaScript's safe-integer limit cannot be represented exactly as Node numbers. The JSON request cap is 1 MiB, including cover-image byte arrays.

**Wait for playback readiness before sending state-dependent controls.** Observe a playing snapshot with advancing position before seeking. Capabilities can be incomplete during startup. After a command, observe events or poll until the expected state appears.

Use the language-specific check commands before distributing a build. Offline checks cover wrapper loading, argument conversion, lifecycle and error handling. **Validate account access, DRM and audio on your target runtime separately.**
