//! ULID generation: 48-bit millisecond timestamp plus 80 random bits,
//! Crockford base32, 26 characters, lexicographically sortable.

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Generates a fresh ULID for the current time.
#[must_use]
pub fn ulid() -> String {
    let random: [u8; 10] = rand::random();
    encode(crate::now_ms().max(0).unsigned_abs(), random)
}

/// Returns true when `value` has the shape of a ULID.
#[must_use]
pub fn is_ulid(value: &str) -> bool {
    value.len() == 26
        && value
            .bytes()
            .all(|byte| ALPHABET.contains(&byte.to_ascii_uppercase()))
        && value.as_bytes()[0] <= b'7'
}

fn encode(time_ms: u64, random: [u8; 10]) -> String {
    let mut bytes = [0_u8; 16];
    bytes[..6].copy_from_slice(&time_ms.to_be_bytes()[2..]);
    bytes[6..].copy_from_slice(&random);
    let mut value = u128::from_be_bytes(bytes);
    let mut out = [b'0'; 26];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("Crockford alphabet is ASCII")
}

#[cfg(test)]
mod tests {
    use super::{encode, is_ulid, ulid};

    #[test]
    fn encodes_known_timestamp_prefix() {
        // The ULID spec example 01ARZ3NDEK... decodes to 1469922850259 ms.
        let value = encode(1_469_922_850_259, [0; 10]);
        assert_eq!(&value[..10], "01ARZ3NDEK");
        assert!(is_ulid(&value));
    }

    #[test]
    fn later_timestamps_sort_after_earlier_ones() {
        let earlier = encode(1_000, [0xFF; 10]);
        let later = encode(1_001, [0; 10]);
        assert!(earlier < later);
    }

    #[test]
    fn generated_ids_are_well_formed_and_unique() {
        let first = ulid();
        let second = ulid();
        assert!(is_ulid(&first));
        assert_ne!(first, second);
    }
}
