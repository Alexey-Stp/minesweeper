# Minesweeper

A clone of classic Windows Minesweeper written in Rust with [`eframe`/`egui`](https://github.com/emilk/egui).
Everything is in [`src/main.rs`](src/main.rs). There are no image assets: the bevels, seven-segment counters
and grid are vector-drawn, and the symbols are Unicode emoji.

## Run

```sh
cargo run --release
```

Requires a recent stable Rust toolchain (edition 2024). On Linux, eframe also needs the system windowing and
OpenGL development libraries; see the [egui README](https://github.com/emilk/egui#demo) for the package list.

## How to play

| Input | Action |
| --- | --- |
| Left click | Reveal a cell. The first click is always safe, and so are its neighbours. |
| Right click | Cycle a covered cell: flag 🚩 → question mark `?` → blank |
| Middle click, or left + right together | Chord: on a revealed number whose flags are all placed, reveal its other neighbours |
| Smiley button or <kbd>F2</kbd> | New game |
| **Game** menu | New game, Beginner (9×9, 10 mines), Intermediate (16×16, 40), Expert (30×16, 99) |

The left counter shows mines minus flags (it can go negative). The right counter is the elapsed time, which
starts on your first reveal and stops when the game ends.

When you hit a mine, it turns red, the other mines are shown 💣 and misplaced flags are marked ❌.
When you clear every safe cell, the remaining mines are flagged for you and the smiley puts on sunglasses 😎.

## Tests

```sh
cargo test
```

The game rules (mine placement, flood fill, flag cycling, chording, win and loss) are covered by unit tests
at the bottom of `src/main.rs`.
