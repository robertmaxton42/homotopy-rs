// The fast maps are the only place the std collections may be named (`clippy.toml` disallows them
// elsewhere, so that every map uses this hasher).
#[allow(clippy::disallowed_types)]
use std::collections::{HashMap, HashSet};
use std::{
    hash::{BuildHasherDefault, Hash, Hasher},
    ops::BitXor,
};

/// The Fx hash (as in `rustc-hash` 1.1 on a 64-bit target) with a 64-bit state on every target.
///
/// `rustc_hash::FxHasher` keeps a `usize`, which is 32 bits on wasm32: there it uses the 32-bit
/// constant, and `finish` returns a value whose top 32 bits are zero, so hashbrown's 7-bit control
/// tag (the top bits of the hash) is always 0 and the iteration order differs from native. This
/// type gives the same hashes on wasm32 as on a 64-bit native build.
#[derive(Clone, Copy)]
pub struct Hasher64 {
    hash: u64,
}

const SEED: u64 = 0x517c_c1b7_2722_0a95;

impl Default for Hasher64 {
    #[inline]
    fn default() -> Self {
        Self { hash: 0 }
    }
}

impl Hasher64 {
    #[inline]
    fn add_to_hash(&mut self, i: u64) {
        self.hash = self.hash.rotate_left(5).bitxor(i).wrapping_mul(SEED);
    }
}

impl Hasher for Hasher64 {
    #[inline]
    fn write(&mut self, mut bytes: &[u8]) {
        let mut hash = *self;
        while bytes.len() >= 8 {
            hash.add_to_hash(u64::from_ne_bytes(bytes[..8].try_into().unwrap()));
            bytes = &bytes[8..];
        }
        if bytes.len() >= 4 {
            hash.add_to_hash(u64::from(u32::from_ne_bytes(
                bytes[..4].try_into().unwrap(),
            )));
            bytes = &bytes[4..];
        }
        if bytes.len() >= 2 {
            hash.add_to_hash(u64::from(u16::from_ne_bytes(
                bytes[..2].try_into().unwrap(),
            )));
            bytes = &bytes[2..];
        }
        if let Some(&b) = bytes.first() {
            hash.add_to_hash(u64::from(b));
        }
        self.hash = hash.hash;
    }

    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(u64::from(i));
    }

    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }

    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FastHasher = BuildHasherDefault<Hasher64>;
#[allow(clippy::disallowed_types)]
pub type FastHashMap<K, V> = HashMap<K, V, FastHasher>;
#[allow(clippy::disallowed_types)]
pub type FastHashSet<K> = HashSet<K, FastHasher>;

// backport unstable feature "build_hasher_simple_hash_one"
// https://github.com/rust-lang/rust/issues/86161
// !! use for debugging only !!
pub fn hash_one<T: Hash>(x: T) -> u64 {
    let mut hasher: Hasher64 = Default::default();
    x.hash(&mut hasher);
    hasher.finish()
}
