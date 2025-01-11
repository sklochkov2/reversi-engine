# reversi-engine
A reversi game engine created as part of learning Rust. It can play games
against itself or against other engines with the help of the reversi server.
Besides, it can generate opening books for its own use in order to be able
to make moves calculated at higher depths during tghe initial stages of a game.

## Compiling

```bash
cargo build --release
```

## Usage

```bash
$ ./target/release/reversi-engine --help
Usage: reversi-engine [OPTIONS]

Options:
  -a, --api-url <API_URL>
          API base URL, e. g. http://example.com:8080/ [default: ]
  -p, --player-uuid <PLAYER_UUID>
          Player UUID as provided by server API [default: ]
  -s, --search-depth <SEARCH_DEPTH>
          Search depth [default: 8]
  -b, --book-path <BOOK_PATH>
          Opening book path [default: ]
  -g, --generate-book
          Whether to generate an opening book
  -f, --full-depth <FULL_DEPTH>
          When generating an opening book, how deeply to evaluate all moves [default: 5]
  -k, --k-partial-depth <K_PARTIAL_DEPTH>
          When generating an opening book, how deeply to analyze main lines [default: 7]
  -h, --help
          Print help
  -V, --version
          Print version
```

### Playing against itself

```bash
./target/release/reversi-engine -s 8

xz -d < examples/opening-book.json.xz > opening-book.json
./target/release/reversi-engine -s 8 -b opening-book.json
```

### Playing a game on a Reversi server
```bash
./target/release/reversi-engine -a "http://127.0.0.1:8000/" -s 6 -b ./reversi-book.json -p "<player uuid>"
```

### Generating an opening book
```bash
./target/release/reversi-engine --generate-book -k 5 -p 5 -s 10 -b ./5_10.json
```
