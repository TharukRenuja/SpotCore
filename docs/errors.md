# Errors and recovery

Things can go wrong: a cookie expires, a device disappears, or Spotify limits a request. Service and control calls return `Result`, so your app can handle those cases. Match the errors you can recover from and keep a fallback arm for future variants.

```rust
use spotcore::{Error, Paging, SearchKind, SpotCore};

async fn search(spotify: &SpotCore) -> Result<(), Error> {
    match spotify.search().query(SearchKind::Tracks, "Miles Davis", Paging::default()).await {
        Ok(page) => println!("{} tracks", page.items.len()),
        Err(Error::Http { status: 429, retry_after }) => {
            eprintln!("Rate limited; retry after {retry_after:?} seconds");
        }
        Err(Error::NotAuthenticated) => eprintln!("Ask the user to authenticate again"),
        Err(error) => return Err(error),
    }
    Ok(())
}
```

| Error | App response |
| --- | --- |
| `NotAuthenticated` | Obtain fresh credentials or use optional login |
| `NativeAuthenticationRequired` | Soloist has not confirmed authentication; inspect registration/pairing |
| `NativeStartupFailed` | Inspect both native and Web fallback errors |
| `InvalidInput` | Correct the request; do not retry unchanged |
| `Unsupported` | Disable the action for this mode/device |
| `AccountRestricted` | Explain account/backend eligibility |
| `DrmUnavailable` | Check Widevine and AAC using the engine guide |
| `NoCompatibleEngine` | Inspect candidate failures and setup requirements |
| `Http` | Handle status and optional retry delay |
| `EventsLagged` | Refresh playback state |
| `Disconnected` / `Timeout` | Re-establish connectivity and inspect current state |

## Rate limits and writes

Authenticated HTTP remembers numeric `Retry-After` values on 429 per Spotify host within the API instance. Calls during that cooldown return the remaining delay without another request. Other hosts stay independent. Cooldowns also apply to writes; do not confirm an optimistic change when the SDK returned an error.

There is no automatic replay of a mutation after an ambiguous acknowledgement or a transport failure. Before retrying a playlist edit, transfer or playback action, inspect state where possible. The recommendation endpoint's persistent 429 has not been established to be ordinary transient throttling; do not loop indefinitely.

## Permissions and recovery

`NativeAuthenticationRequired` means a running Soloist daemon has not confirmed
Spotify authentication. It does not mean the service login expired or prove the
Soloist key is invalid. `NativeStartupFailed { native, fallback }` preserves the
native error when subsequent Web startup/DRM validation fails. Bindings expose
these as `native_authentication_required` and `native_startup_failed` codes.

Refresh state, then read `spotify.capabilities()` before presenting playback actions. Restrictions can change between the check and execution, so still handle errors. A missing or region-restricted catalog item is distinct from an empty successful search.

HTTP-only sessions do not expose playback events. Do not reconnect in a tight loop on `next_event()` returning `Unsupported`; use app-managed polling. Explicitly shut down owned sessions before rebuilding them. Browser reload recovery does not replay pending writes; engine fallback happens during initialization, before playback writes.
