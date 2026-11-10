# Authentication and accounts

Your app can obtain a Spotify session cookie and pass it to `.session_cookie(sp_dc)`. Treat that cookie as a credential: keep it out of source code, logs and command arguments. SDK cookie-backed access obtains and renews access tokens; a revoked or expired cookie requires new credentials. A 401 invalidates cached authentication; mutations are not automatically replayed.

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

Interactive login is optional and requires a browser host. `.profile_dir(path)` reuses a browser profile; `.data_dir(root)` places SDK state under an explicit root. `SPOTCORE_DATA_DIR` overrides the default root. Use `.app_id("my-app")?` for an application subdirectory under the default root. Default paths follow each OS's application-data conventions.

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
    });
    accounts.switch_with(profile).await?;
    accounts.close().await?;
    Ok(())
}
```

Use the canonical Spotify username, not the display name. Selection verifies identity and closes the previous owned session; it does not automatically transfer playback. Each account has separate browser/native data and an exclusive session lease.

`switch_with()` does not persist the supplied profile to AccountStore, although a launched browser can retain its own cookies. For saved accounts, use `add(profile)`, `list()`, `switch_to(username)` and `forget(username)`. `close()` stops the selected session.

AccountStore keeps readable account metadata separate from secrets. Its default credential-store integration uses the OS credential vault; it does not silently fall back to plaintext if the vault fails. Apps can provide a `CredentialStore` through `AccountStore::with_credential_store` and pass it to `AccountManager::with_store`. Legacy profiles are migrated; filesystem permissions alone are not encryption. See `examples/multi_account.rs` for a complete workflow.

## AccountManager reference

All asynchronous operations return `Result<…, Error>`; accessors return borrowed state without network requests.

| Method | Use |
| --- | --- |
| `new(paths)` | Create the default AccountStore and OS credential-store integration |
| `with_store(paths, store)` | Use an app-provided store/vault configuration |
| `engine`, `browser_role`, `web_options`, `device_name` | Configure subsequent account sessions; same meanings as builder options |
| `list().await` | Read `Vec<AccountMetadata>` without opening the secret vault |
| `add(profile).await` | Save metadata and credentials; selection verifies the identity |
| `switch_to(username).await` | Load and select a saved account |
| `switch_with(profile).await` | Select app-supplied credentials without saving the profile |
| `login(username).await` | Interactive login for the canonical account in its isolated browser profile |
| `prefer_native(enabled)` | Set the native preference for subsequent opens; defaults to true, but still requires the native feature, saved key and fresh Premium verification |
| `active_account()` | Optional canonical username of the selected account |
| `session()` / `session_mut()` | Optional borrowed selected SpotCore session |
| `close().await` | Stop the selected session and release its lease |
| `forget(username).await` | Stop it if active, remove saved account credentials and its SDK account data |

Forgetting an account is a local data operation, not Spotify account deletion. It does not sign out an unrelated official Spotify application. Do not delete directories directly while a managed session holds a lease.

## Credential-store integration

`AccountProfile::new(canonical_username)` initializes empty optional credentials and metadata. Its fields include display name, tier, tier-check timestamp, `WebCredentials` and `SoloistCredentials`. Tier metadata alone does not authorize native playback: the launch verifies current account state.

`WebCredentials` holds optional `sp_dc`, access token and expiry; `SoloistCredentials` holds an optional API key. `AccountSecrets` groups both for a vault. A custom `CredentialStore` implements synchronous `load(reference)`, `save(reference, secrets)` and `delete(reference)` returning `std::io::Result`. Implementations must never include secret contents in errors; OS prompts/blocking work must stay off the UI thread.

```rust
use spotcore::{AccountManager, AccountStore, CredentialStore, Paths};
use std::sync::Arc;

fn accounts_with_vault(vault: Arc<dyn CredentialStore>) -> Result<AccountManager, spotcore::Error> {
    let paths = Paths::for_app("my-app")?;
    let store = AccountStore::with_credential_store(paths.accounts(), vault);
    AccountManager::with_store(paths, store)
}
```

The app implements `CredentialStore` using its existing vault; the SDK supplies `OsCredentialStore` for the default OS integration. References identify vault entries, not serialized credentials to display to users.

For direct storage use, `AccountStore` exposes `new`, `with_credential_store`, `base_dir`, `account_path`, `load_metadata`, `save_profile`, `load_profile`, `delete_account`, `list_accounts`, `record_tier` and `resolve_session`. These are synchronous filesystem/vault operations returning I/O results. `resolve_session` returns an optional `ResolvedSession`; profile helpers include `has_web`, `native_available`, `playback_mode` and `tier_is_stale(max_age_ms)`. AccountManager is preferable for async app workflows because it also owns account-session shutdown, identity checks and leases.

Legacy plaintext profile migration preserves data if vault storage fails. The readable metadata file is not encrypted; secrets are stored separately. A custom credential store determines its own security properties. Account-specific browser cookies can persist independently in the browser profile.
