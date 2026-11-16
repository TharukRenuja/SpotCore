# Node.js / TypeScript binding

This is a native N-API addon, not WebAssembly. It uses `napi-rs` to await the same Rust session worker used by the other wrappers. No browser-side SDK is provided.

```sh
python3 scripts/build_node.py
python3 scripts/check_bindings.py node
```

Requires Rust, Node.js 18+ and a TypeScript compiler for the declaration check. No npm dependencies are required to run the wrapper. **Build the `.node` binary under `generated/` for the deployment OS and architecture**, then distribute it with the wrapper.

```typescript
import { Session, SpotCoreError } from './bindings/node';

const spotify = new Session();
try {
  const info = await spotify.login(spDc, {
    appId: 'my-node-app', deviceName: 'My Music App', playback: false,
  });
  const results = await spotify.searchTracks('Miles Davis', 10, 0);
  for (const track of results.items) console.log(track.name, track.uri);
} catch (error) {
  if (error instanceof SpotCoreError) console.error(error.code, error.message);
  else throw error;
} finally {
  await spotify.shutdown();
}
```

The [CLI example](examples/search.cjs) reads a private raw cookie file. `playback=false` means HTTP services without a browser. Set `playback=true` for local audio and supply the optional native key as the third `login` argument. `SessionInfo.engine` and `fallbackReason` report the result of SDK selection.

All methods return Promises. `nextEvent()` awaits one playback event. `pollEvent()` reads the existing event buffer without waiting or network polling. Use one event consumer. Camel-case DTOs and declarations are in `index.d.ts`. Async SDK/protocol failures reject with `SpotCoreError` (`code`, `message`). Cancelling application waiting does not undo a native request, and writes must not be retried blindly.

Use an Electron main process or a Node-enabled host and expose application IPC to its UI. Browser pages and sandboxed renderers cannot load this addon. **Package and verify the native addon for your target runtime.** Electron integration also needs application-owned IPC and packaging. Existing Web engine, DRM and Soloist prerequisites still apply. Use typed starter calls for common tasks and `invoke()` for the full service/playback operation inventory.

## Full domain access

Use the shared `invoke` interface for library, playlists, Home, catalog, podcasts, devices, social/Jam and advanced playback. See the [language examples](../../docs/bindings.md#service-and-playback-operations) and [complete operation reference](../../docs/binding-operations.md). These results retain SDK/Spotify JSON fields. Existing permissions and backend limitations apply.
