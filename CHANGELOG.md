# Changelog

## v1.0.2 — 2026-10-23

### Fixed

- `due` no longer comes due at every tick when a source's interval is huge (`Duration::MAX` wrapped to a negative number), and no longer overflows on a finish time far in the past.

### Changed

- The bad paths are covered by tests: an `open` under a file or on a folder, a schema that fails, a stale or out-of-range `reader`, a `commit` whose write fails, and a runner whose listener or inbox goes away. Line coverage is 97.2%, up from 95.4%.
- The README shows the PITO MD wordmark at its Licence section and a line to ask questions on Discord.
- The gate and the releases run outside GitHub, and the README drops its CI badge.

## v1.0.1 — 2026-10-14

### Added

- Docs at docs.pitomd.com: install on each platform, ten guides from writing a source to testing one, the file formats, and the API reference. Every GitHub Release carries them as `docs-vX.Y.Z.tar.gz`.

## v1.0.0 — 2026-10-14

### Added

- The `numbers` example measures the crate on your machine: `cargo run --release --example numbers`.

### Changed

- pito-sync reaches 1.0.0; the API is the same as 0.1.5.
- The README shows every feature with a chart, a clip or code and its output, and leads with measured numbers: a UI read takes 47 µs while a source writes 10,000 rows back to back.
- The brand now reads PITO MD.

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
