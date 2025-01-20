use std::collections::HashMap;

use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub type MoveMask = u64;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct Position {
    pub black: u64,
    pub white: u64,
    pub white_to_move: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookEntry {
    pub suggested_moves: Vec<MoveMask>,
}

#[derive(Default, Debug)]
pub struct OpeningBook {
    pub entries: HashMap<Position, BookEntry>,
}

impl OpeningBook {
    pub fn insert_position(&mut self, pos: Position, move_mask: MoveMask) {
        self.entries
            .entry(pos)
            .and_modify(|entry| {
                if !entry.suggested_moves.contains(&move_mask) {
                    entry.suggested_moves.push(move_mask);
                }
            })
            .or_insert_with(|| BookEntry {
                suggested_moves: vec![move_mask],
            });
    }

    pub fn get(&self, pos: &Position) -> Option<&BookEntry> {
        self.entries.get(pos)
    }

    pub fn insert_all_rotations(&mut self, pos: Position, move_mask: MoveMask) {
        let mut p = pos;
        let mut m = move_mask;
        for _ in 0..4 {
            self.insert_position(p, m);
            p = rotate_position_90(&p);
            m = rotate_move_90(m);
            self.insert_position(flip_position_vertical(&p), flip_move_vertical(m));
            self.insert_position(flip_position_horizontal(&p), flip_move_horizontal(m));
        }
    }

    // Example serialization/deserialization
    pub fn save_to_file(&self, path: &str) -> std::io::Result<()> {
        let file = std::fs::File::create(path)?;
        let writer = std::io::BufWriter::new(file);
        println!("Saving current book state to file {}", path);
        serde_json::to_writer_pretty(writer, self)?;
        Ok(())
    }

    pub fn load_from_file(path: &str) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let reader = std::io::BufReader::new(file);
        let book = serde_json::from_reader(reader)?;
        Ok(book)
    }
}

fn rotate90(b: u64) -> u64 {
    let mut rotated: u64 = 0;
    for row in 0..8 {
        for col in 0..8 {
            let from_index = row * 8 + col;
            let to_index = col * 8 + (7 - row);
            if (b >> from_index) & 1 == 1 {
                rotated |= 1 << to_index;
            }
        }
    }
    rotated
}

pub fn rotate_position_90(pos: &Position) -> Position {
    Position {
        black: rotate90(pos.black),
        white: rotate90(pos.white),
        white_to_move: pos.white_to_move,
    }
}

fn rotate_move_90(m: MoveMask) -> MoveMask {
    rotate90(m)
}

pub fn flip_position_vertical(pos: &Position) -> Position {
    Position {
        black: flip_vertical(pos.black),
        white: flip_vertical(pos.white),
        white_to_move: pos.white_to_move,
    }
}

pub fn flip_position_horizontal(pos: &Position) -> Position {
    Position {
        black: flip_horizontal(pos.black),
        white: flip_horizontal(pos.white),
        white_to_move: pos.white_to_move,
    }
}

fn flip_move_vertical(m: MoveMask) -> MoveMask {
    flip_vertical(m)
}

fn flip_move_horizontal(m: MoveMask) -> MoveMask {
    flip_horizontal(m)
}

fn flip_vertical(x: u64) -> u64 {
    let mut result = 0;
    for row in 0..8 {
        let row_bits = (x >> (8 * row)) & 0xFF;
        result |= row_bits << (8 * (7 - row));
    }
    result
}

fn flip_horizontal(x: u64) -> u64 {
    let mut result = 0;
    for row in 0..8 {
        let row_bits = ((x >> (8 * row)) & 0xFF) as u8;
        let reversed = reverse_byte(row_bits);
        result |= (reversed as u64) << (8 * row);
    }
    result
}

fn reverse_byte(mut b: u8) -> u8 {
    b = (b << 4) | (b >> 4);
    b = ((b & 0xCC) >> 2) | ((b & 0x33) << 2);
    b = ((b & 0xAA) >> 1) | ((b & 0x55) << 1);
    b
}

impl Serialize for OpeningBook {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.entries.len()))?;
        for (pos, entry) in &self.entries {
            let key = format!("{},{},{}", pos.black, pos.white, pos.white_to_move as u8);
            map.serialize_entry(&key, entry)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for OpeningBook {
    fn deserialize<D>(deserializer: D) -> Result<OpeningBook, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct BookVisitor;

        impl<'de> Visitor<'de> for BookVisitor {
            type Value = OpeningBook;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a map from string -> BookEntry")
            }

            fn visit_map<M>(self, mut access: M) -> Result<OpeningBook, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut book = OpeningBook::default();

                while let Some((key, entry)) = access.next_entry::<String, BookEntry>()? {
                    let parts: Vec<&str> = key.split(',').collect();
                    if parts.len() != 3 {
                        return Err(serde::de::Error::custom("invalid key format"));
                    }
                    let black = parts[0].parse::<u64>().map_err(serde::de::Error::custom)?;
                    let white = parts[1].parse::<u64>().map_err(serde::de::Error::custom)?;
                    let white_to_move_num =
                        parts[2].parse::<u8>().map_err(serde::de::Error::custom)?;
                    let pos = Position {
                        black,
                        white,
                        white_to_move: white_to_move_num != 0,
                    };
                    book.entries.insert(pos, entry);
                }

                Ok(book)
            }
        }

        deserializer.deserialize_map(BookVisitor)
    }
}

/*
fn main() -> std::io::Result<()> {
    let mut book = OpeningBook::default();

    let initial_pos = Position {
        black: 0x0000000810000000,
        white: 0x0000001008000000,
        white_to_move: false,
    };

    let row = 2;
    let col = 3;
    let suggested_move_mask = move_mask_from_rc(row, col);

    book.insert_all_rotations(initial_pos, suggested_move_mask);

    book.save_to_file("opening_book.json")?;

    let loaded_book = OpeningBook::load_from_file("opening_book.json")?;

    if let Some(entry) = loaded_book.get(&initial_pos) {
        println!("Found entry. Suggested moves:");
        for (i, &m) in entry.suggested_moves.iter().enumerate() {
            println!("  Move #{i}: 0x{m:016X}");
        }
    } else {
        println!("No entry found for this position.");
    }

    Ok(())
}*/
