# Contributing

Bug fixes, documentation improvements and binding fixes are welcome. Keep each change focused so it’s easy to review.

## Report a bug

Include your OS and architecture, SpotCore revision, account tier, playback backend and the steps to reproduce it. Tell us what you expected and what happened instead. For binding issues, include the language and runtime version too.

Share only sanitized errors. Never include cookies, API keys, access tokens, account-store files or raw authenticated network traces. Offline reproductions are especially helpful; a bug report doesn’t need your Spotify credentials.

## Work on the SDK

Use Rust 1.89 or newer, Python 3 for the operation generator, and Node.js for the JavaScript bridge tests. Start with the [installation guide](docs/installation.md). Browser playback also needs the [engine setup](docs/engines.md); offline SDK checks don’t require a Spotify account or playback runtime.

Make your change on a branch. Fix shared behavior in the Rust SDK rather than duplicating Spotify logic in a language wrapper. Add a regression check for changed behavior, and update the relevant [developer guide](docs/README.md) when an API or setup step changes.

## Run checks

Build the launcher before running SDK tests:

```sh
cargo fmt --all --check
cargo build -p spotcore-engine --locked
cargo clippy -p spotcore -p spotcore-core -p spotcore-auth -p spotcore-web -p spotcore-native --all-targets --all-features --locked -- -D warnings
cargo test -p spotcore -p spotcore-core -p spotcore-auth -p spotcore-web -p spotcore-native --all-features --locked
cargo check -p spotcore --examples --all-features --locked
python3 scripts/generate_ffi_operations.py --check
node crates/web/tests/js/bridge.test.cjs
node crates/web/tests/js/services.test.cjs
```

These commands check the SDK without compiling the optional OS web-engine helper. Whole-workspace checks also build `host/system`; Linux needs WebKitGTK/GTK development packages as described in [engine setup](docs/engines.md). CI checks desktop platforms and Rust feature combinations. Live account tests are separate and opt-in.

## CI runs

Code changes run the SDK, FFI, launcher and Node checks together on Linux. Documentation changes get lightweight link and generated-reference checks without compiling the SDK. Feature branches use pull-request checks instead of duplicate push and PR runs.

Before a version release or after platform-specific changes, open **Actions → CI → Run workflow** and enable **full**. This adds Windows/macOS, Rust 1.89 and OS web-engine checks. Artifact packaging stays separate on the development repository. Runs have timeouts, superseded runs are cancelled, and failure diagnostics expire after three days.

## Change a binding

The shared binding layer lives in `crates/spotcore-ffi`; wrappers, tests and examples live under `bindings/`. Edit the source rather than generated files. If service or playback signatures change, regenerate the operation adapters and reference:

```sh
python3 scripts/generate_ffi_operations.py
python3 scripts/generate_ffi_operations.py --check
```

For generated language wrappers, use `scripts/generate_bindings.py` and follow the matching [language guide](bindings/README.md). For C, C++, Go and Node, use their existing wrapper sources and check commands. Generated libraries and build outputs stay out of Git.

## Submit your change

Open a pull request with a short explanation of the problem, what changed and which checks you ran. Mention anything you couldn’t verify, especially platform-specific or live playback behavior. Keep unrelated cleanup in a separate change, and preserve existing third-party attribution.
