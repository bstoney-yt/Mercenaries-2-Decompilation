//! A fast hasher for the backend's small look-up keys (material ids,
//! pipeline keys), which are looked up several times for every draw.
//! The standard library's hasher resists crafted keys, which these never
//! are, at several times the cost.
//!
//! The mixing step (rotate, xor, multiply by an odd constant) is the
//! well-known "Fx" scheme. Provenance: invented (renderer plumbing).

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// A [`HashMap`] hashed with [`FastHasher`].
pub(crate) type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<FastHasher>>;

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

/// Hashes word by word: rotate the state, mix the word in, multiply.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FastHasher {
    hash: u64,
}

impl FastHasher {
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(SEED);
    }
}

impl Hasher for FastHasher {
    fn write(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for c in &mut chunks {
            self.add(u64::from_le_bytes(c.try_into().expect("8 bytes")));
        }
        let rest = chunks.remainder();
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            word[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    fn hash<T: Hash + ?Sized>(t: &T) -> u64 {
        BuildHasherDefault::<FastHasher>::default().hash_one(t)
    }

    #[test]
    fn equal_keys_hash_equal_and_nearby_ids_apart() {
        assert_eq!(hash(&(7u32, true)), hash(&(7u32, true)));
        let hashes: std::collections::HashSet<u64> = (0u32..10_000).map(|i| hash(&i)).collect();
        assert_eq!(hashes.len(), 10_000);
        assert_ne!(hash(&"abc"), hash(&"abd"));
    }

    #[test]
    fn a_map_finds_what_it_holds() {
        let mut map = FastMap::default();
        for i in 0u32..5_000 {
            map.insert(i, i * 3);
        }
        assert!((0u32..5_000).all(|i| map[&i] == i * 3));
        assert!(!map.contains_key(&5_000));
    }
}
