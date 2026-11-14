//! Task ids: 6 lowercase base36 characters, unique across the vault.

use std::collections::HashSet;

pub const ID_LEN: usize = 6;
const ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// True if `s` is a well-formed task id (without the `^`).
pub fn is_valid_id(s: &str) -> bool {
    s.len() == ID_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// Generate a random id not contained in `taken`, and insert it into `taken`.
pub fn generate_id(taken: &mut HashSet<String>) -> String {
    loop {
        let id: String = (0..ID_LEN)
            .map(|_| ALPHABET[fastrand::usize(..ALPHABET.len())] as char)
            .collect();
        if taken.insert(id.clone()) {
            return id;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validity() {
        assert!(is_valid_id("a1b2c3"));
        assert!(is_valid_id("000000"));
        assert!(!is_valid_id("A1B2C3"));
        assert!(!is_valid_id("a1b2c"));
        assert!(!is_valid_id("a1b2c3d"));
        assert!(!is_valid_id("a1-2c3"));
        assert!(!is_valid_id("ąbcdef"));
    }

    #[test]
    fn generated_ids_are_valid_and_unique() {
        let mut taken = HashSet::new();
        for _ in 0..20_000 {
            let before = taken.len();
            let id = generate_id(&mut taken);
            assert!(is_valid_id(&id), "{id}");
            assert_eq!(taken.len(), before + 1);
        }
    }

    #[test]
    fn generate_avoids_taken() {
        // Fill a deterministic RNG's first outputs into the taken set and make sure
        // the generator skips them.
        fastrand::seed(7);
        let mut probe = HashSet::new();
        let first = generate_id(&mut probe);
        fastrand::seed(7);
        let mut taken: HashSet<String> = [first.clone()].into_iter().collect();
        let next = generate_id(&mut taken);
        assert_ne!(next, first);
    }
}
