//! SHA-256 and its canonical lowercase hexadecimal representation.
//! Callers own input framing, versions, namespaces, prefixes and digest validation.

use sha2::{Digest, Sha256};

/// Hash exactly these bytes, without framing or a domain prefix.
pub fn sha256_hex(bytes: &[u8]) -> String {
    encode_sha256(&Sha256::digest(bytes).into())
}

/// Encode an already-computed SHA-256 digest without hashing it again.
pub fn encode_sha256(digest: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answers_match() {
        for (input, expected) in [
            (
                b"".as_slice(),
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                b"abc".as_slice(),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ] {
            assert_eq!(sha256_hex(input), expected);
        }
    }

    #[test]
    fn digest_encoding_retains_leading_zeroes_and_every_byte_value() {
        for start in (0..256).step_by(32) {
            let bytes = std::array::from_fn(|i| (start + i) as u8);
            let expected: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            assert_eq!(encode_sha256(&bytes), expected);
            assert_eq!(encode_sha256(&bytes).len(), 64);
        }
        assert_eq!(encode_sha256(&[0; 32]), "0".repeat(64));
        assert_eq!(encode_sha256(&[255; 32]), "f".repeat(64));
    }
}
