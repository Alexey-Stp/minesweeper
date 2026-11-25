# Minesweeper

A clone of classic Windows Minesweeper: one player clears a board of covered cells without revealing a mine.

## Language

### The board

**Board**:
The grid of cells in one game, sized by its difficulty.
_Avoid_: grid, field, minefield

**Cell**:
One square of the board. It holds a mine or it doesn't, and it is covered, marked or open.
_Avoid_: tile, square

**Neighbours**:
The up to eight cells touching a cell, diagonals included.
_Avoid_: adjacent cells, surrounding cells

**Number**:
On an open cell, how many of its neighbours hold mines. An open cell with no neighbouring mines is **blank**.
_Avoid_: count, hint

**Mine layout**:
Where the mines go on a board, laid at the first reveal. Normally random, keeping the first revealed cell and (when there is room) its neighbours clear; it can also be fixed in advance.
_Avoid_: minefield, mine placement

**Difficulty**:
A preset board size and mine count: Beginner, Intermediate or Expert.
_Avoid_: level, mode

### Playing

**Move**:
One reveal, mark or chord made on the board.

**Reveal**:
Opening a covered cell. Revealing a blank cell also reveals its neighbours, spreading through blank cells (**flood fill**).
_Avoid_: open, uncover, click

**Mark**:
A flag or question mark on a covered cell; marking cycles flag → question mark → unmarked. A flag protects its cell from being revealed; a question mark does not.
_Avoid_: tag

**Chord**:
Revealing every unflagged neighbour of a number at once, allowed when exactly that many neighbours are flagged.
_Avoid_: area reveal, double click

**Status**:
Where a game stands: Ready (no reveal yet, so no mines laid), Playing, Won (every safe cell revealed) or Lost (a mine revealed).

**Mines left**:
The mine count minus the flags placed; negative when there are more flags than mines.
_Avoid_: mine counter, flags left

**Timer**:
Whole seconds since the first reveal, frozen when the game ends.
_Avoid_: clock

### The mouse

**Gesture**:
One use of the mouse on the board, from the first button press until every button is up. Releasing the left button reveals, pressing the right button alone marks, and holding both buttons (or the middle one) chords on the first release.
_Avoid_: click (for the whole gesture)

**Board input**:
The rules that turn gestures into moves, and that decide which cells look pushed in while a button is held.

**Face**:
The button above the board that starts a new game and shows how play is going: it gasps while a button is held on the board, and changes when the game is won or lost.
_Avoid_: smiley, reset button
