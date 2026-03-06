Implement real NIP-01 cryptographic signing in the Rust Nostr adapter by replacing placeholder crypto logic. Focus only on local cryptographic correctness (`pubkey`, `id`, `sig`) and key parsing.

### Objective
Replace placeholder operations with:
1. secp256k1 x-only public key derivation (BIP-340 format).
2. NIP-01 event ID generation (SHA256 over canonical serialized event array).
3. BIP-340 Schnorr signature generation over the 32-byte event ID.

### Scope
In scope:
1. `crates/adapters/src/nostr/mod.rs` crypto/key-parsing changes.
2. `Cargo.toml` workspace dependency additions.
3. `crates/adapters/Cargo.toml` dependency wiring.
4. Single CLI call-site adjustment in `crates/cli/src/commands/run.rs` for new constructor signature.
5. Unit tests for parsing, serialization, signing, and consistency.

Out of scope:
1. Relay transport/protocol fixes (WebSocket vs HTTP, payload shape changes).
2. Publisher trait or domain crate API changes.
3. Key generation/management flows.
4. Any NIP beyond NIP-01 except decoding `nsec` input.

### Required Implementation
1. Use `k256 = { version = "0.13", features = ["schnorr"] }` and `bech32 = "0.11"`.
2. Add `parse_secret_key(raw: &str) -> Result<k256::schnorr::SigningKey>`:
1. Trim leading/trailing whitespace.
2. If input starts with `nsec1`, decode Bech32 and require:
1. HRP exactly `nsec`.
2. Payload exactly 32 bytes.
3. Reject all other HRPs/lengths/invalid encodings.
3. Otherwise, parse as hex and require exactly 64 hex chars -> 32 bytes.
4. Construct `SigningKey`; reject invalid scalar (zero or out of range).
3. Update `NostrPublisher`:
1. Replace stored raw secret with `signing_key: Option<SigningKey>` (`None` for disabled).
2. Change `new(secret_key, relays)` to return `Result<Self>` and fail fast on invalid key.
3. Keep `disabled()` infallible with `signing_key: None`.
4. Update event creation:
1. `create_event` must accept `created_at: i64` (injected by caller for testability).
2. Canonical serialization must use `serde_json::to_string(&(0, &pubkey, created_at, kind, &tags, content))`.
3. `id = sha256(serialized_bytes)` and hex-encode lowercase 64 chars.
4. `sig` must be valid BIP-340 signature over `id` bytes and hex-encode lowercase 128 chars.
5. `pubkey` must be lowercase 64-char x-only hex.
5. `publish` passes `OffsetDateTime::now_utc().unix_timestamp()` to `create_event`.
6. CLI call site (`crates/cli/src/commands/run.rs:232`):
1. Update from `Ok(NostrPublisher::new(...))` to `NostrPublisher::new(...)` to match new return type.
2. No other CLI logic changes.
7. Avoid panics for normal error paths; invalid keys and signing failures must be returned as errors with clear messages.

### Acceptance Criteria
1. `NostrPublisher::new` returns `Err` immediately for invalid keys (empty, malformed hex, wrong hex length, wrong `nsec` HRP, wrong `nsec` payload length, invalid scalar).
2. Both valid hex and valid `nsec` forms of the same key produce the same derived public key.
3. For a fixed `(secret_key, created_at, kind, tags, content)`, `create_event` yields mutually consistent fields:
1. Recomputed ID from event fields equals `event.id`.
2. Signature verifies against `event.id` using `event.pubkey`.
4. JSON escaping edge cases (quotes/newlines/backslashes/unicode) produce correct ID via `serde_json` canonical serialization.
5. Existing Nostr adapter tests continue passing after updates.
6. Add a deterministic reference-vector test:
1. Use one official BIP-340 signing vector from the Bitcoin BIPs test vectors.
2. Assert exact expected pubkey and signature for that vector.
3. This test is required to prove deterministic signing behavior for the chosen API path.

### Files To Modify
1. `Cargo.toml` (workspace root): add `k256` and `bech32` to `[workspace.dependencies]`.
2. `crates/adapters/Cargo.toml`: add workspace deps for `k256` and `bech32`.
3. `crates/adapters/src/nostr/mod.rs`: key parsing, constructor, pubkey derivation, event creation/signing, test updates/additions.
4. `crates/cli/src/commands/run.rs`: one-line constructor return-type adjustment.

### Testing Requirements
1. Add/update unit tests for:
1. hex + `nsec` equivalence.
2. invalid key rejection matrix.
3. deterministic reference vector (official BIP-340 vector).
4. event consistency (`pubkey`, `id`, `sig`) with fixed timestamp.
5. JSON escaping behavior.
2. Keep tests local/unit-level; no relay/network acceptance tests for this task.