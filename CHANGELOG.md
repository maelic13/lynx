# Changelog

All notable changes to Lynx are documented in this file.

## [1.2.1] - 2026-05-25

### Changed

- Made timed-search allocation more conservative at fast controls by assuming
  a slightly longer remaining game and lowering the hard-stop cap. This reduces
  avoidable time forfeits while leaving fixed-depth and fixed-movetime search
  behavior unchanged.

### Added

- Added time-manager regression coverage for fast sudden-death clocks,
  fixed `movetime`, side-to-move clock selection, explicit `movestogo`,
  minimum allocation, and unbounded fixed-depth searches.
- Added UCI option/parser regression coverage for default `go`, invalid search
  limit values, invalid `setoption` values, and preservation of previous
  engine option values after rejected input.

## [1.2.0] - 2026-05-25

### Added

- Added UCI `MultiPV` option and basic sequential MultiPV root analysis output.
- Added `tbhits` reporting in UCI search info.
- Added Syzygy tablebase load summaries that report WDL and DTZ file counts and
  the largest loaded tablebase cardinality.
- Added root Syzygy move ranking through Fathom's DTZ root API, with WDL root
  fallback when DTZ tables are unavailable.
- Added regression coverage for tablebase file counting, Fathom root move
  decoding, local Syzygy root probing when tables are present, root repetition
  exposure, `tb_hits`, and the MultiPV search path.

### Changed

- Changed root tablebase behavior to filter and search tablebase-correct root
  moves instead of immediately returning a single zero-node root move.
- Improved Stockfish compatibility for Syzygy UCI defaults and practical root
  tablebase usage.
- Passed repetition state into root DTZ probing so root tablebase ranking can
  account for repeated positions.
- Specialized color-specific attack lookup and reduced repeated bitboard
  queries in evaluation hot paths for a modest NPS improvement.

### Fixed

- Fixed root move restriction handling so tablebase-filtered and
  helper-thread root move lists are actually respected by the root search.

## [1.1.0] - 2026-05-25

### Added

- Added optional Syzygy tablebase support using the vendored Fathom probe
  library, with UCI options for path, probe depth, probe limit, and
  fifty-move-rule handling.
- Added tests for Syzygy option parsing, Fathom result decoding, disabled-path
  probing, root move conversion, and malformed path handling.

### Changed

- Improved search selectivity and speed with faster check detection, stronger
  stale-TT internal iterative reduction, deeper null-move reductions,
  check-aware pruning, and adjusted late move reductions.
- Improved tactical reliability by continuing quiescence search through check
  evasions instead of falling back to static evaluation while in check.
- Stored ProbCut lower bounds in the transposition table to improve move
  ordering and cut reuse.

### Fixed

- Reduced the risk of illegal or tactically losing search choices from
  over-pruning checking moves and check evasions.

## [1.0.2] - 2026-05-25

### Changed

- Removed Intel macOS release assets from the GitHub release build matrix.
  macOS release binaries are now Apple Silicon/ARM64 only.
- Clarified release asset CPU targets and local native build behavior in the
  README.

## [1.0.1] - 2026-05-23

Patch release focused on UCI tournament reliability.

### Fixed

- Delayed `bestmove` emission for completed `go ponder ...` searches until
  `ponderhit`, `stop`, or `quit`, matching the Stockfish-style UCI control flow
  where a completed ponder result is retained but not reported early.
- Delayed `bestmove` emission for completed `go infinite` searches until
  `stop` or `quit`.
- Preserved the `ponder` flag when parsing combined `go ponder infinite`
  commands.
- Flushed UCI `uciok`, `readyok`, and `bestmove` replies immediately for
  pipe-based GUIs and tournament managers.

### Added

- Regression tests covering UCI ponder and infinite bestmove-release behavior.
- Search and option-parser coverage for ponderhit conversion, infinite search
  limits, and temporary limit reset behavior.

## [1.0.0] - 2026-05-22

Initial Lynx release.

### Implemented

- Custom Rust board representation based on bitboards, mailbox lookup, fixed
  move lists, incremental make/unmake, and Zobrist hashing.
- Complete legal move generation for standard chess, including castling,
  en passant, promotions, checks, pins, repetitions, the fifty-move rule, and
  insufficient material detection.
- Perft and board correctness test coverage for common reference positions and
  special move edge cases.
- Search benchmark support through the UCI `bench [depth]` command.
- Release benchmark and performance tests for board operations.
- `cargo bench --bench board` board implementation benchmark for cross-version
  and cross-engine comparison.
- UCI `Threads` option with Lazy SMP-style persistent-worker search and packed
  shared-transposition-table support up to 1024 threads.
- Epoch-tracked UCI command control with prioritized `quit`, asynchronous
  `stop`/`ponderhit`, serialized idle `isready` handling, and in-order EOF
  shutdown for redirected UCI sessions.
- Threaded root result selection and fixed-depth helper root diversification for
  stronger Lazy SMP behavior during both timed searches and benchmarks.
- Capture-focused qsearch with bounded check evasions and SEE-based pruning.
- Transposition table, pawn cache, correction history, main history, capture
  history, continuation history, killers, and countermoves.
- Advanced search features: PVS, aspiration windows, null-move pruning,
  ProbCut, singular extensions, futility pruning, late move pruning, late move
  reductions, SEE move ordering/pruning, and quiescence search.
- Handcrafted tapered evaluation with material, PeSTO-style piece-square tables,
  pawn structure, passed pawns, mobility, rook activity, outposts, threats,
  king safety, draw scaling, and winning-king push.
- Release profile tuning with fat LTO, one codegen unit, and `panic = "abort"`.

### Completed

- Implemented all board, search, and evaluation logic inside the engine without
  external chess crates.
- Reworked UCI engine configuration around the custom board and searcher.
- Updated the built-in `bench` command to use the current UCI options,
  including `Threads`.
- Hardened transposition-table resizing so failed large `Hash` allocations keep
  the current table instead of leaving search without TT storage.
- Hardened the shared transposition table with full-key validation to avoid
  cross-thread partial-key collisions.
- Updated qsearch to avoid full legal move regeneration outside check and to
  reuse transposition-table results in tactical leaf searches.
- Made `ponderhit` reset the active search clock after ponder search converts
  to normal thinking.
- Made helper-thread creation fail gracefully if the OS cannot create every
  requested worker.
- Kept UCI node-limited searches on the main search path to preserve the
  requested node limit when `Threads` is greater than one.
- Rebranded the package, binary, UCI identification, release workflow, and
  documentation for Lynx.
- Set the initial Lynx release version to `1.0.0`.
- Configured GitHub release assets to publish portable x86-64 and ARM64 builds,
  plus optimized AVX2 and AVX-512 builds where supported.
- Expanded documentation for UCI support, benchmarking, testing, and engine
  internals.
