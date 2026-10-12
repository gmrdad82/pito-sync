# Changelog

## v0.1.5 — 2026-10-12

### Added

- The `feed` example: a source on its own thread, a secret handed in after start, resyncs, a failed pass that keeps its rows, and a UI that only reads.
- MIT licence, CI on Linux and macOS, and a GitHub Release for every tag.

### Changed

- The README is written for anyone who wants to use the crate, with an animated run of the example.
- The tests run with cargo-nextest, and the doctests with cargo test.

## v0.1.4 — 2026-10-05

### Added

- `spawn_checked` opens the database on the calling thread and returns the open error, or moves the connection into the runner thread; `spawn` is unchanged.

## v0.1.3 — 2026-10-04

### Added

- `pass_once` runs one pass on the calling thread (mark started, run the pass, record a failure), and the runner wraps exactly that with its two `Changed` sends.

## v0.1.2 — 2026-10-03

### Changed

- An explicit `Resync` always runs, even right after a pass.

## v0.1.1 — 2026-10-03

### Changed

- A failed pass records its finish, the runner announces each pass's start, and the first pass runs at once.

## v0.1.0 — 2026-10-03

### Added

- The local-SQLite sync layer: one database per app, opened private with WAL and rebuilt when its version changes, sources on their own threads, `Changed` and `Resync`.
