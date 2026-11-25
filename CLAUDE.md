# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```sh
cargo run --release                                          # play
cargo test                                                   # all tests
cargo test game::tests::timer                                # tests whose path contains the filter
cargo test board_input::tests::middle_button_chords -- --exact   # one test
cargo clippy --all-targets                                   # warning-free today; keep it so
cargo fmt --check                                            # default rustfmt config
```

## Toolchain gotchas

- `cargo` lives in `~/.cargo/bin`, which may be missing from PATH: prepend it (`export PATH="$HOME/.cargo/bin:$PATH"`). The VS Code tasks in `.vscode/tasks.json` already do.
- On Windows, build with the MSVC toolchain (`stable-x86_64-pc-windows-msvc`). The GNU toolchain fails on eframe's `windows-*` crates, which need MinGW's `dlltool` and `as`. On Linux, eframe needs the system windowing and OpenGL dev libraries (see the egui README).
- Give a git worktree its own `--target-dir`. Cargo keys this package by its path inside the workspace, so a worktree sharing this checkout's `target/` leaves builds that cargo then treats as fresh for the other tree's sources.

## Architecture

Three modules, each with a small interface that its tests go through:

- `src/game.rs` holds the rules and knows nothing of the UI. `Game` keeps its fields private: moves in (`reveal`, `cycle_mark`, `chord`), views out (`view`, `status`, `mines_left`, `elapsed_seconds`). `Board` owns the board's shape: size, row-major cell numbering, `neighbours`. Mines are laid on the first reveal from a `MineLayout`: `Random { mines, seed }` in play, keeping the first revealed cell (and its neighbours, when there is room) clear; `Fixed(cells)` in tests. Time is an input (`now` on `reveal`, `chord`, `elapsed_seconds`), so a game is deterministic given its inputs.
- `src/board_input.rs` holds the mouse rules and knows nothing of egui. `BoardInput` owns the gesture state machine (Idle → Press → Chord → Spent) and is the one place that decides whether a button counts as held.
- `src/main.rs` is the egui side: window sizing, `Layout` (screen rects, point → cell), `read_pointer`, and painting. Everything is vector-drawn; symbols are emoji from egui's bundled font.

One frame:

```text
MinesweeperApp::ui                          reads Instant::now() once
  game_area
    read_pointer                            egui input → PointerFrame
    BoardInput::frame(game, pointer, now)   applies any finished move → HoldFeedback
    paint_*                                 game.view, counters, face_glyph(status, holding)
```

## Tests

Tests sit at the bottom of each file and use only that module's interface, even though Rust lets the child `tests` module see private fields.

- `game.rs`: build a known board with `game_from(&["*..", "...", "..*"])` (`*` is a mine; it uses `MineLayout::Fixed`) and assert what the player sees with `render(&game)` grids (legend on `render`). Use a seeded `MineLayout::Random` for property tests.
- `board_input.rs`: drive the `Mouse` harness (`move_to`, `press`, `release`, `press_and_release`) on its fixed 3×3 board.
- The egui wiring (`read_pointer`, `game_area`) has no tests: check changes there by running the game.

## Language

`GLOSSARY.md` holds the game's canonical terms (reveal, mark, chord, gesture, mine layout, …) and the words to avoid. Use its terms in code, docs, test names and commit messages, and add an entry when you introduce a new concept.
