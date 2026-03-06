---
artifact: acceptance-pass
loop: 2
project: issue-3
backend: claude(opus)
role: qa
created_at: 2026-03-05T04:36:26Z
---

# QA: PASS

## Manual Testing

Verified the implementation by reading all modified source files and examining the complete `git diff master...HEAD`:

1. **Dependencies**: `k256 = { version = "0.13", features = ["schnorr"] }` and `bech32 = "0.11"` added to workspace root `Cargo.toml:57-58` and wired in `crates/adapters/Cargo.toml:38-39`.

2. **Key parsing** (`crates/adapters/src/nostr/mod.rs:18-81`): `parse_secret_key` handles both hex (64-char) and `nsec1` Bech32 formats. Trims whitespace, validates HRP, payload length, hex validity, and scalar range. Returns `Result<SigningKey>` with clear error messages.

3. **Constructor** (`mod.rs:124-138`): `NostrPublisher::new` returns `Result<Self>` and fails fast on invalid keys. `disabled()` (`mod.rs:141-148`) remains infallible with `signing_key: None`.

4. **Event creation** (`mod.rs:151-180`): `create_event` accepts `created_at: i64`, uses `serde_json::to_string(&(0, &pubkey, created_at, kind, &tags, content))` for canonical serialization, computes SHA-256 for the ID, signs with BIP-340 Schnorr using deterministic zero aux randomness, and hex-encodes all fields to the correct lengths (pubkey: 64, id: 64, sig: 128).

5. **Publish** (`mod.rs:250`): Passes `OffsetDateTime::now_utc().unix_timestamp()` to `create_event`.

6. **CLI call-site** (`crates/cli/src/commands/run.rs:233`): Updated from `Ok(NostrPublisher::new(...))` to `NostrPublisher::new(secret_key.expose_secret(), ...)` matching the new `Result` return type. Added `use secrecy::ExposeSecret` import.

## Automated Tests

All **64 tests pass** across the workspace (0 failures):

- **47 adapter tests** (including 11 Nostr-specific):
  - `test_parse_secret_key_hex` — hex parsing works
  - `test_parse_secret_key_nsec` — nsec Bech32 parsing works
  - `test_parse_secret_key_hex_and_nsec_equivalence` — same key from both formats produces same pubkey
  - `test_parse_secret_key_invalid_matrix` — rejects empty, wrong-length hex, non-hex chars, wrong HRP (`npub`), wrong payload lengths (31/33 bytes), zero scalar
  - `test_create_event` — event field lengths are correct
  - `test_event_fields_are_consistent` — recomputed ID matches `event.id`; signature verifies against `event.pubkey` and `event.id` using `PrehashVerifier`
  - `test_json_escaping_canonicalization` — quotes, backslashes, unicode in content produce correct ID
  - `test_bip340_reference_vector_0` — official BIP-340 test vector (secret key `0x03`, message `0x00*32`) produces exact expected pubkey and signature
  - `test_disabled_publisher` / `test_no_relays_configured` / `test_publish_success` — existing integration tests pass
- **14 domain tests** — all pass
- **3 CLI integration tests** — all pass

## Acceptance Criteria Verification

| # | Criterion | Status | Evidence |
|---|-----------|--------|----------|
| 1 | `NostrPublisher::new` returns `Err` for invalid keys | **PASS** | `test_parse_secret_key_invalid_matrix` covers empty, malformed hex, wrong hex length, wrong `nsec` HRP, wrong `nsec` payload length, invalid scalar (zero) |
| 2 | Hex and `nsec` forms of same key produce same pubkey | **PASS** | `test_parse_secret_key_hex_and_nsec_equivalence` asserts verifying key bytes match |
| 3 | Fixed inputs produce consistent `id`, `pubkey`, `sig` | **PASS** | `test_event_fields_are_consistent` recomputes ID from canonical serialization and verifies signature with `PrehashVerifier` |
| 4 | JSON escaping edge cases produce correct ID | **PASS** | `test_json_escaping_canonicalization` uses quotes, backslashes, unicode and verifies ID |
| 5 | Existing Nostr adapter tests continue passing | **PASS** | `test_disabled_publisher`, `test_no_relays_configured`, `test_publish_success` all pass |
| 6 | Deterministic BIP-340 reference vector test | **PASS** | `test_bip340_reference_vector_0` uses official BIP-340 vector #0 (key `0x03`) and asserts exact pubkey `f9308a...` and signature `e90783...` |
