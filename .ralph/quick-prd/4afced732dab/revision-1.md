## Summary

Replace placeholder cryptographic operations in the Nostr adapter with real NIP-01 compliant event signing. This means: deriving the public key from a secp256k1 secret key, computing the event ID as SHA256 of the canonical serialization, and producing a valid Schnorr signature over the event ID. Without this, relays reject all published events.

## Acceptance Criteria

1. Public key is derived from the secret key using secp256k1 (x-only/BIP-340 format, 32-byte hex).
2. Event ID is SHA256 of `[0,"<pubkey>",<created_at>,<kind>,<tags>,"<content>"]` using proper JSON serialization via `serde_json` (not manual `format!`).
3. Signature is a valid BIP-340 Schnorr signature over the 32-byte event ID hash.
4. Events produced by `create_event` have correct and mutually consistent `id`, `pubkey`, and `sig` fields that pass NIP-01 validation. Relay transport behavior is out of scope (see Out of Scope).
5. Secret key input supports both raw 64-character hex and `nsec1...` (bech32-encoded) formats. Input is trimmed of leading/trailing whitespace before parsing. The bech32 HRP must be exactly `nsec` and the decoded payload must be exactly 32 bytes; any other input is rejected.
6. `NostrPublisher::new` returns `Result<Self>` (instead of bare `Self`). Invalid secret keys cause an immediate error at construction time with a descriptive message. The single CLI call site (`crates/cli/src/commands/run.rs:232`) already operates in a `Result`-returning function and will propagate the error via `?`.
7. Existing tests continue to pass (with updated assertions); new tests verify deterministic signing against a known test vector.
8. No changes to the `Publisher` trait or domain crate. Workspace-root `Cargo.toml` dependency additions are allowed; CLI call-site updates are limited to adding `?` to the `NostrPublisher::new` call.

## Technical Approach

**Crate choice:** Use `k256` (pure-Rust, no system dependencies, audited) with the `schnorr` feature for BIP-340 Schnorr signing. Add `bech32` for nsec decoding. These avoid the C dependency of `libsecp256k1` and keep the build simple.

**Secret key parsing — `parse_secret_key` helper:**

Add a function `parse_secret_key(raw: &str) -> Result<k256::schnorr::SigningKey>` that:
1. Trims leading/trailing whitespace from the input.
2. If the trimmed input starts with `nsec1`, decodes it using `bech32::decode`. Validates that the HRP is exactly `nsec` and that the decoded data payload is exactly 32 bytes. Returns an error otherwise.
3. Otherwise, treats the input as a 64-character hex string. Decodes it and validates the result is exactly 32 bytes.
4. Constructs and returns a `k256::schnorr::SigningKey` from the 32-byte secret key material. Returns an error if the bytes are not a valid secp256k1 scalar (e.g., zero or >= curve order).

**Changes to `NostrPublisher` struct and constructors:**

- Replace the `secret_key: SecretString` field with `signing_key: Option<k256::schnorr::SigningKey>` (`None` when disabled).
- Change `new(secret_key: SecretString, relays: Vec<String>) -> Self` to `new(secret_key: SecretString, relays: Vec<String>) -> Result<Self>`. Inside, call `parse_secret_key(secret_key.expose_secret())` and store the result in `signing_key: Some(...)`. Return the parse error if the key is invalid.
- `disabled()` remains infallible and sets `signing_key: None`.

**Changes to `NostrPublisher::derive_pubkey`:**

- Extract the x-only public key from the stored `SigningKey` via `signing_key.verifying_key()`.
- Return the 32-byte hex-encoded x-only public key.
- Panics (via `.expect()`) if called when `signing_key` is `None`; this is only reachable from `create_event`, which is only called from `publish`, which early-returns when `enabled` is false — so the disabled path never reaches here.

**Changes to `NostrPublisher::create_event`:**

- Accept an additional `created_at: i64` parameter (instead of calling `OffsetDateTime::now_utc()` internally). The caller (`publish`) passes `OffsetDateTime::now_utc().unix_timestamp()`. This makes the function deterministically testable.
- Use `serde_json::to_string(&(0, &pubkey, created_at, kind, &tags, content))` to produce the canonical serialized array. This handles JSON escaping of special characters correctly.
- SHA256-hash the serialized array to produce the 32-byte event ID.
- Sign the event ID bytes using `signing_key.sign_prehash(&id_bytes)` (the `sign_prehash` method on `k256::schnorr::SigningKey` signs a pre-hashed 32-byte message per BIP-340, using RFC 6979 deterministic nonce generation, so signatures are fully deterministic for the same key + message).
- Set `id` to the 64-char hex-encoded event ID.
- Set `sig` to the 128-char hex-encoded Schnorr signature.

**Crypto API specifics (k256 0.13):**

- `k256::schnorr::SigningKey::from_bytes(slice)` — construct from 32-byte secret key.
- `signing_key.verifying_key()` — returns the `VerifyingKey` (x-only public key).
- `signing_key.sign_prehash(msg_bytes)` — BIP-340 Schnorr signature over a 32-byte pre-hashed message. Uses deterministic (RFC 6979) nonce derivation, ensuring identical inputs always produce identical signatures. Returns `Result<Signature, Error>`.
- `verifying_key.verify_prehash(msg_bytes, &signature)` — used in tests to verify.
- The `sign_prehash` / `verify_prehash` methods require the `k256` crate's `schnorr` feature, which is specified in the dependency.

**Relay publish path (`publish_to_relay`):** No changes. The current HTTP-POST transport with `["EVENT", "<json>"]` payload shape is out of scope for this feature (see Out of Scope).

**CLI call-site change (`crates/cli/src/commands/run.rs:232`):** Update from `Ok(NostrPublisher::new(...))` to `NostrPublisher::new(...)` (it already returns `Result` now, and the function returns `Result<NostrPublisher>` so the `?` propagation works naturally).

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` (workspace root) | Add `k256 = { version = "0.13", features = ["schnorr"] }` and `bech32 = "0.11"` to `[workspace.dependencies]`. This is a manifest-only change, not a logic change to code outside adapters. |
| `crates/adapters/Cargo.toml` | Add `k256 = { workspace = true }` and `bech32 = { workspace = true }` to `[dependencies]` |
| `crates/adapters/src/nostr/mod.rs` | Replace `derive_pubkey` and `create_event` implementations; add `parse_secret_key` helper; change struct field from `secret_key: SecretString` to `signing_key: Option<k256::schnorr::SigningKey>`; change `new()` to return `Result<Self>`; add `created_at` parameter to `create_event`; update `publish` to pass timestamp; update and add tests |
| `crates/cli/src/commands/run.rs` | Line 232: change `Ok(NostrPublisher::new(secret_key, config.nostr.relays.clone()))` to `NostrPublisher::new(secret_key, config.nostr.relays.clone())` (remove the `Ok(...)` wrapper since `new` now returns `Result`) |

## Testing Strategy

1. **Deterministic signing test:** Use a known secret key (e.g., hex `"0000...0001"` or a published NIP-01 test vector). Call `create_event` with a fixed `created_at` timestamp and known content. Assert that the resulting `pubkey`, `id`, and `sig` match expected values. Use `k256::schnorr::VerifyingKey::verify_prehash` to confirm the signature is valid over the event ID bytes.

2. **nsec parsing test:** Encode a known 32-byte secret key as both hex and `nsec1...` bech32. Call `parse_secret_key` with each and verify both produce the same public key.

3. **Secret key validation tests:** Verify that `parse_secret_key` returns an error for: empty string, invalid hex, wrong-length hex, bech32 with wrong HRP (e.g., `npub1...`), bech32 with wrong payload length, whitespace-only input. Verify that leading/trailing whitespace around a valid hex key is accepted.

4. **JSON escaping test:** Call `create_event` with content containing quotes, newlines, backslashes, and unicode. Recompute the expected event ID externally (SHA256 of the serde_json canonical form) and verify it matches. This confirms `serde_json` serialization handles edge cases correctly.

5. **`new` returns error on invalid key:** Call `NostrPublisher::new(SecretString::new("not-a-valid-key".into()), vec![])` and assert it returns `Err`.

6. **Existing tests updated:**
   - `test_disabled_publisher` — no changes needed (calls `disabled()`, not `new`).
   - `test_no_relays_configured` — update to use a valid 64-char hex key and add `.unwrap()` to the `new` call.
   - `test_publish_success` — same: use a valid key, `.unwrap()` the `new` call.
   - `test_create_event` — use a known key and fixed `created_at`, assert deterministic `id`, `pubkey`, and `sig` values. Verify signature with `VerifyingKey::verify_prehash`.

7. **Timestamp injection for determinism:** Because `create_event` now takes `created_at: i64` as a parameter, tests pass a fixed value directly. No mocking or clock injection framework is needed.

## Out of Scope

- WebSocket relay transport (the current HTTP-POST approach in `publish_to_relay` is unchanged). The `["EVENT", "<json_string>"]` payload shape is a known deviation from standard Nostr relay protocol (which expects `["EVENT", <json_object>]` over WebSocket); fixing this is a separate transport-layer concern and is not addressed here.
- NIP-19 `npub` display or NIP encodings beyond `nsec` input parsing
- Key generation or key management
- Relay authentication (NIP-42)
- Encrypted DMs or any NIP beyond NIP-01
- Changes to the `Publisher` trait or domain crate logic
- End-to-end relay acceptance testing (criterion 4 from the prior spec has been narrowed to local cryptographic correctness; relay acceptance depends on transport fixes that are out of scope)