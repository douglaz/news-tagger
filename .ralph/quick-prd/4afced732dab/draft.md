## Summary

Replace placeholder cryptographic operations in the Nostr adapter with real NIP-01 compliant event signing. This means: deriving the public key from a secp256k1 secret key, computing the event ID as SHA256 of the canonical serialization, and producing a valid Schnorr signature over the event ID. Without this, relays reject all published events.

## Acceptance Criteria

1. Public key is derived from the secret key using secp256k1 (x-only/BIP-340 format, 32-byte hex)
2. Event ID is SHA256 of `[0,"<pubkey>",<created_at>,<kind>,<tags>,"<content>"]` using proper JSON serialization (not manual `format!`)
3. Signature is a valid BIP-340 Schnorr signature over the 32-byte event ID hash
4. Events published to relays pass NIP-01 validation (id, pubkey, sig all correct)
5. Secret key input supports both raw hex and `nsec1...` (bech32-encoded) formats
6. Existing tests continue to pass; new tests verify deterministic signing against a known test vector
7. No changes to the `Publisher` trait or any code outside the adapters crate

## Technical Approach

**Crate choice:** Use `k256` (pure-Rust, no system dependencies, already audited). Specifically `k256::schnorr` for BIP-340 Schnorr signing. Add `bech32` for nsec decoding. These avoid the C dependency of `libsecp256k1` and keep the build simple.

**Changes to `NostrPublisher::derive_pubkey` (`crates/adapters/src/nostr/mod.rs:79-84`):**
- Parse the secret key (hex or bech32-nsec) into a `k256::schnorr::SigningKey`
- Derive the x-only public key via `signing_key.verifying_key()`
- Return the 32-byte hex-encoded public key

**Changes to `NostrPublisher::create_event` (`crates/adapters/src/nostr/mod.rs:46-76`):**
- Use `serde_json::to_string` to produce the canonical `[0,"<pubkey>",<created_at>,<kind>,<tags>,"<content>"]` array (instead of manual `format!` which doesn't properly handle JSON escaping)
- SHA256-hash the serialized array to get the event ID (reuse existing `sha2` dependency)
- Sign the 32-byte event ID hash with `k256::schnorr::SigningKey::sign_raw` (BIP-340)
- Set `sig` to the 64-byte hex-encoded signature

**nsec handling:** Add a helper function `parse_secret_key(raw: &str) -> Result<SigningKey>` that:
- If input starts with `nsec1`, decode with bech32 and extract the 32-byte secret key
- Otherwise, treat as 64-char hex string
- Return a `k256::schnorr::SigningKey`

**Store the parsed `SigningKey` at construction** (in `NostrPublisher::new`) instead of re-parsing on every event, keeping `SecretString` for input only.

## Files & Modules

| File | Change |
|---|---|
| `Cargo.toml` (workspace) | Add `k256 = { version = "0.13", features = ["schnorr"] }` and `bech32 = "0.11"` to `[workspace.dependencies]` |
| `crates/adapters/Cargo.toml` | Add `k256 = { workspace = true }` and `bech32 = { workspace = true }` to `[dependencies]` |
| `crates/adapters/src/nostr/mod.rs` | Replace `derive_pubkey` and `create_event` implementations; add `parse_secret_key` helper; change `secret_key: SecretString` field to `signing_key: Option<k256::schnorr::SigningKey>` (None for disabled); update `new()` and `disabled()` constructors; update tests |

## Testing Strategy

1. **Deterministic signing test:** Use a known NIP-01 test vector (well-known secret key → expected pubkey, event ID, and valid signature). Verify `create_event` produces matching `id`, `pubkey`, and a valid `sig`.
2. **nsec parsing test:** Verify that both hex and `nsec1...` encoded keys produce the same `SigningKey` / public key.
3. **JSON escaping test:** Verify event ID is correct when content contains special characters (quotes, newlines, backslashes, unicode).
4. **Existing tests:** `test_disabled_publisher`, `test_no_relays_configured`, `test_publish_success`, `test_create_event` — update assertions as needed (e.g., `test_create_event` should use a known key and verify deterministic output).
5. **Signature verification in test:** Use `k256::schnorr::VerifyingKey::verify_raw` in tests to confirm the signature is actually valid, not just non-empty.

## Out of Scope

- WebSocket relay support (current HTTP approach is unchanged)
- NIP-19 `npub` display or other NIP encodings beyond `nsec`
- Key generation or key management
- Relay authentication (NIP-42)
- Encrypted DMs or any NIP beyond NIP-01
- Changes to the domain crate or `Publisher` trait