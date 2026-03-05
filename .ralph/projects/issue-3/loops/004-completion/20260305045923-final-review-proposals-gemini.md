---
artifact: final-review-proposals
loop: 4
project: issue-3
backend: gemini
role: final_reviewer
created_at: 2026-03-05T04:59:23Z
---

# Final Review: AMENDMENTS

## Amendment: MISSING_HEX_PREFIX_HANDLING

### Problem
The `parse_secret_key` function in `crates/adapters/src/nostr/mod.rs` does not explicitly handle or document the case where a hex string is prefixed with `0x`. While many hex libraries support this, the specification requires either an `nsec1` prefix for Bech32 or a raw hex string. Relying on implicit library behavior for `0x` can lead to ambiguity or future breakage if the library changes. The implementation should be explicit about what it accepts.

The current implementation:
```rust
// crates/adapters/src/nostr/mod.rs

// ... in parse_secret_key
    } else {
        // Fallback to hex decoding
        let mut decoded = [0u8; 32];
        hex::decode_to_slice(trimmed_key, &mut decoded)
            .map_err(|e| anyhow!("Failed to decode hex secret key: {}", e))?;
        SigningKey::from_bytes(&decoded).map_err(|e| anyhow!("Invalid secret key: {}", e))
    }
```
This directly passes the trimmed key to `hex::decode_to_slice`. If the key is `0x...`, this will fail, which is good, but it would be better to provide a more specific error message guiding the user to remove the prefix.

### Proposed Change
Add an explicit check in `parse_secret_key` to detect and reject hex keys with a `0x` prefix, providing a clear error message.

```diff
--- a/crates/adapters/src/nostr/mod.rs
+++ b/crates/adapters/src/nostr/mod.rs
@@ -194,6 +194,9 @@
             .map_err(|e| anyhow!("Invalid Bech32-encoded nsec key: {}", e))?;
         SigningKey::from_bytes(&bytes).map_err(|e| anyhow!("Invalid secret key: {}", e))
     } else {
+        if trimmed_key.starts_with("0x") {
+            return Err(anyhow!("Hex secret key should not be prefixed with '0x'"));
+        }
         // Fallback to hex decoding
         let mut decoded = [0u8; 32];
         hex::decode_to_slice(trimmed_key, &mut decoded)

```
This change makes the parsing stricter and the error more helpful to users who might mistakenly use a common hex prefix.

### Affected Files
- `crates/adapters/src/nostr/mod.rs` - Add explicit rejection of `0x` prefixed hex strings.

---
## Amendment: INCOMPLETE_JSON_ESCAPING_TEST

### Problem
The test `test_create_event_with_special_chars_in_content` in `crates/adapters/src/nostr/mod.rs` only tests for content with quotes. The specification explicitly mentions testing various JSON escaping edge cases, including quotes, newlines, backslashes, and unicode characters. A comprehensive test is needed to ensure the canonical serialization correctly handles all these cases, as this is critical for generating a valid and verifiable event ID.

The current test is insufficient:
```rust
// crates/adapters/src/nostr/mod.rs

#[test]
fn test_create_event_with_special_chars_in_content() {
    let content = "this has \"quotes\"";
    // ...
}
```

### Proposed Change
Expand the test case to include a wider range of special characters that require escaping in JSON strings. This will provide greater confidence in the correctness of the event ID generation.

```diff
--- a/crates/adapters/src/nostr/mod.rs
+++ b/crates/adapters/src/nostr/mod.rs
@@ -428,7 +428,9 @@
 
     #[test]
     fn test_create_event_with_special_chars_in_content() {
-        let content = "this has \"quotes\"";
+        // Test various JSON-special characters to ensure correct serialization and ID generation.
+        let content = "{\"key\":\"value with \\\"quotes\\\" and \\\\backslashes\\\\\"}\nand a newline\t and a tab, plus some unicode: \u{1F602}";
+
         let sk_hex = "0101010101010101010101010101010101010101010101010101010101010101";
         let signing_key = parse_secret_key(sk_hex).unwrap();
         let pubkey = signing_key.verifying_key().x_only_public_key().0;

```

### Affected Files
- `crates/adapters/src/nostr/mod.rs` - Enhance the JSON escaping test to cover more edge cases.
