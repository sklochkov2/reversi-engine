pub fn print_board(white: u64, black: u64, last_move: u64, flips: u64, mark_last_move: bool) {
    let mut res: String = "========\n".to_string();
    for i in 0..8 {
        for j in 0..8 {
            let index = ((7 - i) * 8 + j) as usize;
            let bit = 1u64 << index;
            if mark_last_move && bit == last_move {
                res += "\x1b[41m";
            } else if bit & flips > 0 {
                res += "\x1b[42m";
            }
            if (white & bit) != 0 {
                res += "o";
            } else if (black & bit) != 0 {
                res += "x";
            } else {
                res += ".";
            }
            if (mark_last_move && bit == last_move) || bit & flips > 0 {
                res += "\x1b[0m";
            }
        }
        res += "\n";
    }
    res += "========";
    println!("{}", res);
}

pub fn apply_move_verbose(
    white: u64,
    black: u64,
    move_bit: u64,
    is_white_move: bool,
) -> Result<(u64, u64), &'static str> {
    const DIRECTIONS: [(i32, i32); 8] = [
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];

    let (player, opponent) = if is_white_move {
        (white, black)
    } else {
        (black, white)
    };

    if (player | opponent) & move_bit != 0 {
        return Err("Square already occupied");
    }

    let mut flips = 0u64;

    for &(dx, dy) in DIRECTIONS.iter() {
        let mut current_flips = 0u64;
        let mut x = (move_bit.trailing_zeros() % 8) as i32 + dx;
        let mut y = (move_bit.trailing_zeros() / 8) as i32 + dy;
        let mut found_opponent = false;

        while x >= 0 && x < 8 && y >= 0 && y < 8 {
            let index = (y * 8 + x) as usize;
            let bit = 1u64 << index;

            if (opponent & bit) != 0 {
                current_flips |= bit;
                found_opponent = true;
            } else if (player & bit) != 0 {
                if found_opponent {
                    flips |= current_flips;
                }
                break;
            } else {
                break;
            }

            x += dx;
            y += dy;
        }
    }

    if flips == 0 {
        return Err("Invalid move, no discs flipped");
    }

    let player = player | move_bit | flips;
    let opponent = opponent & !flips;

    let (next_white, next_black) = if is_white_move {
        (player, opponent)
    } else {
        (opponent, player)
    };
    print_board(next_white, next_black, move_bit, flips, true);
    Ok((next_white, next_black))
}
