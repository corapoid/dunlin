//! Token and pairing-code helpers.
//!
//! Device tokens are 256 bits of randomness. Pairing codes are short-lived and
//! human-typeable. Only SHA-256 hashes are stored in the relay database.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("kst_{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn random_pairing_code() -> String {
    let mut rng = rand::rng();
    (0..8)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::{hash_token, random_pairing_code, random_token};

    #[test]
    fn tokens_are_unique_and_hashed() {
        let first = random_token();
        let second = random_token();
        assert_ne!(first, second);
        assert!(first.starts_with("kst_"));
        assert_eq!(hash_token(&first), hash_token(&first));
        assert_ne!(hash_token(&first), hash_token(&second));
    }

    #[test]
    fn pairing_codes_are_typeable() {
        let code = random_pairing_code();
        assert_eq!(code.len(), 8);
        assert!(
            code.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        );
    }
}
