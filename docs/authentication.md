# Authentication and accounts

Your app can obtain a Spotify session cookie and pass it to `.session_cookie(sp_dc)`. **Treat the cookie as a credential.** Keep it out of source code, logs and command arguments. SDK cookie-backed access obtains and renews access tokens. A revoked or expired cookie requires new credentials. A 401 invalidates cached authentication. Mutations are not automatically replayed.

## App and profile isolation

```rust
use spotcore::{Paths, SpotCore};

async fn login() -> Result<(), spotcore::Error> {
    let paths = Paths::for_app("my-music-app")?;
    SpotCore::login(Some(&paths.profile())).await?;
    let spotify = SpotCore::builder()
        .data_dir(paths.root())
        .profile_dir(paths.profile())
        .launch().await?;
    spotify.shutdown().await?;
    Ok(())
}
```

Interactive login is optional and requires a browser host. `.profile_dir(path)` reuses a browser profile. `.data_dir(root)` places SDK state under an explicit root. `SPOTCORE_DATA_DIR` overrides the default root. Use `.app_id("my-app")?` for an application subdirectory under the default root. Default paths follow each OS's application-data conventions.

**Managed login closes automatically after verification.** The dedicated window requests a 520 × 720 size, centered on the available screen and reduced for smaller screens. The window manager may control its final position. Your normal browser windows are not closed.

Apps that save credentials themselves can use the builder's `.login_with(callback).await?` method. The async callback receives a verified HTTP `Api` and returns a `Result<T, spotcore::Error>`. Obtain credentials with `api.web_credentials().await?`. **Return success only after saving those credentials to your vault.** The window closes after callback success. A failed save leaves it open until the user closes it, and is not retried. Plain `.login().await?` verifies the account and closes the window, retaining the isolated browser profile without saving credentials to an app-provided vault. Rebuild Qt or system engine helpers when updating to this managed login flow.

After verifying a browser account, AccountManager captures its `sp_dc` through the local browser protocol and saves it in the protected credential store. Services mode can then close the owned browser and use renewable HTTP services. A successful native startup can also release the browser. Web playback keeps its browser. If a helper cannot export the cookie, the session remains browser-backed. Successful browser login is not proof of browser-free services.

`api.http_services(&paths).await` returns a separate HTTP `Api` and redacted-debug `WebCredentials`. It reads only the Spotify session cookie and obtains HTTP access through the SDK's cookie renewal path. It neither persists credentials nor closes the source browser. Verify `users().me()` against the expected account before switching. AccountManager performs that check before saving the credential or releasing the browser. If HTTP verification cannot complete, it retains the working browser session without saving an unverified cookie. Store the returned credentials only in a vault. Missing, ambiguous, foreign-domain or malformed cookies fail rather than creating an unauthenticated HTTP client.

`SpotCore::connect("http://127.0.0.1:9222").await?` attaches to an existing Spotify browser tab. Use a dedicated profile and a loopback CDP endpoint: the debugger grants broad access to that browser. Attached browsers retain their own credentials and are not closed by SDK shutdown.

## Switch accounts

```rust
use spotcore::{AccountManager, AccountProfile, BrowserRole, Paths, WebCredentials};

async fn select_account(username: &str, sp_dc: String) -> Result<(), spotcore::Error> {
    let mut accounts = AccountManager::new(Paths::for_app("my-music-app")?)?
        .browser_role(BrowserRole::Services);
    let mut profile = AccountProfile::new(username);
    profile.web = Some(WebCredentials {
        sp_dc: Some(sp_dc), access_token: None, expires_at_ms: None,
        client_id: None, app_version: None,
    });
    accounts.switch_with(profile).await?;
    accounts.close().await?;
    Ok(())
}
```

**Use the canonical Spotify username for account selection.** Display names are not account identifiers. Selection verifies identity and closes the previous owned session without starting audio. Preparing an unauthenticated native daemon may activate a matching account-scoped receiver with a paused Connect transfer. Each account has separate browser/native data and an exclusive session lease.

`switch_with()` does not persist the supplied profile to AccountStore, although a launched browser can retain its own cookies. For saved accounts, use `add(profile)`, `list()`, `switch_to(username)` and `forget(username)`. `close()` stops the selected session.

AccountStore keeps readable account metadata separate from secrets. Its default credential-store integration uses the OS credential vault. It does not silently fall back to plaintext if the vault fails. Apps can provide a `CredentialStore` through `AccountStore::with_credential_store` and pass it to `AccountManager::with_store`. Legacy profiles are migrated. Filesystem permissions alone are not encryption. See `examples/multi_account.rs` for a complete workflow.

## AccountManager reference

All asynchronous operations return `Result<…, Error>`. Accessors return borrowed state without network requests.

| Method | Use |
| --- | --- |
| `new(paths)` | Create the default AccountStore and OS credential-store integration |
| `with_store(paths, store)` | Use an app-provided store/vault configuration |
| `engine`, `browser_role`, `web_options`, `device_name` | Configure subsequent account sessions. Same meanings as builder options |
| `list().await` | Read `Vec<AccountMetadata>` without opening the secret vault |
| `add(profile).await` | Save metadata and credentials. Selection verifies the identity |
| `switch_to(username).await` | Load and select a saved account |
| `switch_with(profile).await` | Select app-supplied credentials without saving the profile |
| `login(username).await` | Interactive login for the canonical account in its isolated browser profile |
| `prefer_native(enabled)` | Set the native preference for subsequent opens. Defaults to true, but still requires the native feature, saved key and fresh Premium verification |
| `active_account()` | Optional canonical username of the selected account |
| `session()` / `session_mut()` | Optional borrowed selected SpotCore session |
| `close().await` | Stop the selected session and release its lease |
| `forget(username).await` | Stop it if active, remove saved account credentials and its SDK account data |

Forgetting an account is a local data operation, not Spotify account deletion. It does not sign out an unrelated official Spotify application. Do not delete directories directly while a managed session holds a lease.

## Credential-store integration

`AccountProfile::new(canonical_username)` initializes empty optional credentials and metadata. Its fields include display name, tier, tier-check timestamp, `WebCredentials` and `SoloistCredentials`. Tier metadata alone does not authorize native playback: the launch verifies current account state.

`WebCredentials` holds optional `sp_dc`, access token and expiry. `SoloistCredentials` holds an optional API key. `AccountSecrets` groups both for a vault. A custom `CredentialStore` implements synchronous `load(reference)`, `save(reference, secrets)` and `delete(reference)` returning `std::io::Result`. Implementations must never include secret contents in errors. OS prompts/blocking work must stay off the UI thread.

```rust
use spotcore::{AccountManager, AccountStore, CredentialStore, Paths};
use std::sync::Arc;

fn accounts_with_vault(vault: Arc<dyn CredentialStore>) -> Result<AccountManager, spotcore::Error> {
    let paths = Paths::for_app("my-app")?;
    let store = AccountStore::with_credential_store(paths.accounts(), vault);
    AccountManager::with_store(paths, store)
}
```

The app implements `CredentialStore` using its existing vault. The SDK supplies `OsCredentialStore` for the default OS integration. References identify vault entries, not serialized credentials to display to users.

For direct storage use, `AccountStore` exposes `new`, `with_credential_store`, `base_dir`, `account_path`, `load_metadata`, `save_profile`, `load_profile`, `delete_account`, `list_accounts`, `record_tier` and `resolve_session`. These are synchronous filesystem/vault operations returning I/O results. `resolve_session` returns an optional `ResolvedSession`. Profile helpers include `has_web`, `native_available`, `playback_mode` and `tier_is_stale(max_age_ms)`. AccountManager is preferable for async app workflows because it also owns account-session shutdown, identity checks and leases.

Legacy plaintext profile migration preserves data if vault storage fails. The readable metadata file is not encrypted. Secrets are stored separately. A custom credential store determines its own security properties. Account-specific browser cookies can persist independently in the browser profile.

## Add an account without switching playback

`AccountManager::authenticate_account(cookie)` returns an owned, cancellable future. Pass `None` for managed Spotify login or `Some(cookie)` for cookie verification. Creating the future does not borrow the manager for its lifetime, so the current account can continue handling controls while authentication is polled on the SDK runtime.

The operation uses an isolated temporary profile, verifies the account and exported renewable credentials, and saves the profile before allowing the login window to close. It preserves existing Native credentials/preferences for that identity and does not select the new account, start music or transfer playback. Use `switch_to(id)` to select a stored account; use `login(id)` to reauthenticate a known account in its isolated profile.

Dropping the future cancels pending login work. Temporary profile cleanup is scheduled on the runtime; an OS-profile cleanup failure or shutdown can leave an isolated directory under `pending-logins` for later removal. If cancellation happens during a credential-store write, saving may have completed: list saved accounts before retrying. Await the authentication result before presenting success. Closing the Spotify window is not a successful authentication signal.
