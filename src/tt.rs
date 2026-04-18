//! Transposition table for the search.
//!
//! The table is a direct-mapped array of 16-byte slots. Each slot stores a
//! 64-bit position key and a 64-bit packed data word using Hyatt's "lockless
//! hashing" scheme: the two words are stored as `key ^ data` and `data`, so
//! that a torn write from a concurrent updater naturally causes the XOR
//! round-trip to fail and the probe to report a miss instead of returning
//! corrupted state. All atomics use `Relaxed` ordering - the XOR check is
//! what guarantees internal consistency.
//!
//! The key is not derived from an incremental Zobrist scheme. Instead we
//! recompute it from `(us, them)` on every probe and store via two rounds
//! of splitmix64; this avoids paying per-flip XOR work inside
//! `apply_move_unchecked` (which, in Reversi, can touch many squares per
//! move) at the cost of a handful of extra instructions per node - cheaper
//! than the incremental update was when the engine was last tried with
//! Zobrist hashing.

use std::sync::atomic::{AtomicU64, Ordering};

pub const BOUND_NONE: u8 = 0;
pub const BOUND_EXACT: u8 = 1;
pub const BOUND_LOWER: u8 = 2; // true score >= stored
pub const BOUND_UPPER: u8 = 3; // true score <= stored

// Sentinel "no recorded best move" value. Legal move square indices are
// 0..=63; anything else is treated as absent.
pub const NO_MOVE_SQ: u8 = 64;

/// Packed data returned from a TT probe.
#[derive(Copy, Clone)]
pub struct TTData {
    pub score: i32,
    pub depth: i8,
    pub bound: u8,
    pub move_sq: u8,
}

/// One 16-byte slot. Declared 16-byte aligned so each slot stays within a
/// single cache line pair.
#[repr(align(16))]
pub struct TTSlot {
    word_a: AtomicU64,
    word_b: AtomicU64,
}

impl TTSlot {
    const fn empty() -> Self {
        Self {
            word_a: AtomicU64::new(0),
            word_b: AtomicU64::new(0),
        }
    }
}

pub struct TranspositionTable {
    slots: Box<[TTSlot]>,
    mask: usize,
    age: AtomicU64,
}

impl TranspositionTable {
    pub fn new_mb(mb: usize) -> Self {
        let entry_size = std::mem::size_of::<TTSlot>();
        let requested = (mb * 1024 * 1024) / entry_size;
        // Round DOWN to power-of-two so the modulo is a mask.
        let entries = prev_power_of_two(requested).max(1024);
        let slots: Vec<TTSlot> = (0..entries).map(|_| TTSlot::empty()).collect();
        Self {
            slots: slots.into_boxed_slice(),
            mask: entries - 1,
            age: AtomicU64::new(0),
        }
    }

    /// Bump the age counter. Called once per iterative-deepening root search
    /// so the replacement policy can prefer overwriting stale entries.
    pub fn new_age(&self) {
        self.age.fetch_add(1, Ordering::Relaxed);
    }

    pub fn clear(&self) {
        for s in self.slots.iter() {
            s.word_a.store(0, Ordering::Relaxed);
            s.word_b.store(0, Ordering::Relaxed);
        }
        self.age.store(0, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn probe(&self, key: u64) -> Option<TTData> {
        let idx = (key as usize) & self.mask;
        // Safety: `idx` is masked to be a valid index.
        let slot = unsafe { self.slots.get_unchecked(idx) };
        let a = slot.word_a.load(Ordering::Relaxed);
        let b = slot.word_b.load(Ordering::Relaxed);
        if a == 0 && b == 0 {
            return None;
        }
        if a ^ b != key {
            return None;
        }
        Some(unpack(b))
    }

    #[inline(always)]
    pub fn store(&self, key: u64, score: i32, depth: i8, bound: u8, move_sq: u8) {
        let idx = (key as usize) & self.mask;
        let slot = unsafe { self.slots.get_unchecked(idx) };

        let existing_a = slot.word_a.load(Ordering::Relaxed);
        let existing_b = slot.word_b.load(Ordering::Relaxed);
        let existing_empty = existing_a == 0 && existing_b == 0;
        let existing_key = existing_a ^ existing_b;
        let existing_depth = ((existing_b >> 16) & 0xFF) as i8;
        let existing_age = ((existing_b >> 40) & 0xFF) as u8;

        let cur_age = (self.age.load(Ordering::Relaxed) & 0xFF) as u8;

        // Replacement policy: empty slot wins instantly; same position
        // always overwrites so deeper results supersede shallower; otherwise
        // prefer replacing stale (different-age) entries, or equal-/deeper-
        // depth entries of the current age.
        let same_pos = !existing_empty && existing_key == key;
        let replace = existing_empty
            || same_pos
            || existing_age != cur_age
            || depth as i16 >= existing_depth as i16;

        if !replace {
            return;
        }

        let b = pack(score, depth, bound, move_sq, cur_age);
        let a = key ^ b;
        slot.word_a.store(a, Ordering::Relaxed);
        slot.word_b.store(b, Ordering::Relaxed);
    }
}

#[inline(always)]
fn pack(score: i32, depth: i8, bound: u8, move_sq: u8, age: u8) -> u64 {
    let score_u = score as i16 as u16 as u64;
    let depth_u = depth as u8 as u64;
    let bound_u = bound as u64;
    let move_u = move_sq as u64;
    let age_u = age as u64;
    score_u | (depth_u << 16) | (bound_u << 24) | (move_u << 32) | (age_u << 40)
}

#[inline(always)]
fn unpack(b: u64) -> TTData {
    let score = (b & 0xFFFF) as u16 as i16 as i32;
    let depth = ((b >> 16) & 0xFF) as u8 as i8;
    let bound = ((b >> 24) & 0xFF) as u8;
    let move_sq = ((b >> 32) & 0xFF) as u8;
    TTData {
        score,
        depth,
        bound,
        move_sq,
    }
}

#[inline(always)]
fn prev_power_of_two(n: usize) -> usize {
    if n == 0 {
        1
    } else {
        1usize << (usize::BITS as usize - 1 - n.leading_zeros() as usize)
    }
}

// --------------------------------------------------------------------------
// Hash computation
// --------------------------------------------------------------------------

#[inline(always)]
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline(always)]
pub fn hash_position(us: u64, them: u64) -> u64 {
    // Two full-avalanche splitmix64 mixes, combined asymmetrically so that
    // `hash_position(us, them) != hash_position(them, us)` and the side-to-
    // move is implicitly encoded in the slot.
    let a = mix64(us);
    let b = mix64(them);
    a ^ b.rotate_left(17)
}

// --------------------------------------------------------------------------
// Global TT singleton
// --------------------------------------------------------------------------

use std::sync::OnceLock;

static GLOBAL_TT: OnceLock<TranspositionTable> = OnceLock::new();

pub const DEFAULT_TT_MB: usize = 4;

/// Access the global transposition table, creating it on first use.
pub fn tt() -> &'static TranspositionTable {
    GLOBAL_TT.get_or_init(|| TranspositionTable::new_mb(DEFAULT_TT_MB))
}
