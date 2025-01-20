use once_cell::sync::Lazy;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use reversi_tools::position::*;

#[derive(Clone, Copy)]
pub struct TTEntry {
    pub key: u64,
    pub flag: TTFlag,
    pub value: i32,
    pub best_move: u64,
}

impl Default for TTEntry {
    fn default() -> Self {
        TTEntry {
            key: 0,
            flag: TTFlag::NotFound,
            value: 0,
            best_move: 0
        }
    }
}

#[derive(Clone, Copy)]
pub enum TTFlag {
    Exact,
    AlphaBound,
    BetaBound,
    NotFound,
}

#[derive(Default)]
pub struct TranspositionTable {
    pub entries: Vec<TTEntry>,
    pub size: usize,
}

static ZOBRIST_TABLE: Lazy<[[u64; 2]; 64]> = Lazy::new(|| {
    let mut rng = StdRng::seed_from_u64(123456789);

    let mut table = [[0u64; 2]; 64];
    for cell in 0..64 {
        table[cell][0] = rng.gen();
        table[cell][1] = rng.gen();
    }
    table
});

pub fn compute_zobrist_hash(pos: RichPosition) -> u64 {
    let mut hash = 0u64;

    for cell in 0..64 {
        let mask = 1u64 << cell;
        if pos.white & mask != 0 {
            hash ^= ZOBRIST_TABLE[cell][0];
        } else if pos.black & mask != 0 {
            hash ^= ZOBRIST_TABLE[cell][1];
        }
    }

    hash
}

#[inline]
fn lowest_set_bit(x: u64) -> u64 {
    x & x.wrapping_neg()
}

#[inline]
fn table_pos(x: u64) -> usize {
    x.trailing_zeros() as usize
}

pub fn update_zobrist_hash(pos: RichPosition, hash: u64) -> u64 {
    let color: usize;
    if pos.white_to_move {
        color = 0;
    } else {
        color = 1;
    }
    let mut new_hash: u64 = hash;
    new_hash ^= ZOBRIST_TABLE[table_pos(pos.last_move)][color];
    let mut flipped = pos.flips;
    while flipped != 0 {
        let tmp = lowest_set_bit(flipped);
        flipped &= !tmp;
        new_hash ^= ZOBRIST_TABLE[table_pos(tmp)][color];
    }
    new_hash
}

#[inline]
fn find_index(hash: u64, size: usize) -> usize {
    (hash & (1 << size) - 1) as usize
}

impl TranspositionTable {
    pub fn insert_position(&mut self, hash: u64, eval: i32, kind: TTFlag, mv: u64) {
        //let index = (hash % self.size as u64) as usize;
        let index = find_index(hash, self.size);
        self.entries[index] = TTEntry{
            key: hash,
            flag: kind,
            value: eval,
            best_move: mv
        };
    }

    pub fn probe(&mut self, hash: u64, alpha: i32, beta: i32) -> (i32, u64) {
        //let hash_key: u64 = compute_zobrist_hash(white, black, white_to_move);
        //let index = (hash % self.size as u64) as usize;
        let index = find_index(hash, self.size);
        let entry = self.entries[index];
        if entry.key != hash {
            return (-163840, 0);
        }
        match entry.flag {
            TTFlag::NotFound => {
                (-163840, 0)
            }
            TTFlag::Exact => {
                (entry.value, entry.best_move)
            }
            TTFlag::AlphaBound => {
                if entry.value >= beta {
                    (entry.value, entry.best_move)
                } else {
                    (-163840, 0)
                }
            }
            TTFlag::BetaBound => {
                if entry.value <= alpha {
                    (entry.value, entry.best_move)
                } else {
                    (-163840, 0)
                }
            }
        }
    }

    pub fn new(size: usize) -> Self {
        let entries = vec![TTEntry::default(); 1 << size];
        TranspositionTable { entries, size }
    }
}
