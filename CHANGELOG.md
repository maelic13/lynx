# Changelog

All notable changes to Lynx are documented in this file.

## [1.4.1] - 2026-05-28

Patch release focused on search hotpath profiling and NPS-oriented technical
cleanup after the 1.4.0 TT-move safety work.

### Fixed

- Fixed fast legal validation for recaptures that remove a checking piece, such
  as `...gxf6` after `Nxf6+`, by removing the captured attacker from virtual
  attack bitboards as well as from occupancy.

### Changed

- Replaced clone-and-make based legal validation for TT-shaped and raw UCI
  moves with direct occupancy-based king-safety validation. This preserves the
  strict pseudo-legal checks from 1.4.0 while avoiding a full board clone on TT
  hits.
- Batched Lazy SMP shared node accounting so threaded search no longer performs
  an aggregate atomic node increment on every searched node. Final threaded
  search results still report exact total nodes by summing each thread result.
- Skipped duplicate TT capture scoring in the staged move picker after the TT
  move has already been emitted first.

### Added

- Added a board benchmark workload for direct legal move validation so TT/UCI
  move-validation cost is visible in local profiling.
- Added regression coverage for pseudo-legal pinned moves that must still be
  rejected by the final legal validator.

## [1.4.0] - 2026-05-28

Minor release focused on Stockfish-style transposition-table move safety and
search move ordering.

### Fixed

- Hardened TT/hash move use so stored moves are validated and canonicalized for
  the current board before search, quiescence, PV construction, or ponder
  fallback can use them. This prevents stale, aliased, or concurrently observed
  TT moves from entering searched PVs.
- Fixed UCI PV safety around malformed TT moves by rejecting impossible piece
  movement, moves from the wrong side, friendly-occupied destinations, illegal
  king movement, and malformed special-move encodings before `make_move`.

### Changed

- Reworked the move picker to emit a validated TT move first, then skip the
  duplicate when generated legal captures, quiets, or bad captures are visited.
- Changed UCI move parsing to use the same direct legal-move validator as TT
  move validation, while preserving canonical internal flags for captures,
  castling, en passant, and promotions.
- Added TT-derived ponder fallback from the child position when the searched PV
  does not already contain a ponder move.

### Added

- Added board-level pseudo-legal and legal move validation APIs for raw UCI or
  TT-shaped moves.
- Added regression coverage for friendly-occupied king moves, impossible king
  movement, malformed tournament-derived TT moves, TT-first move picking,
  TT-derived ponder fallback, and UCI PV replay at `Threads=1` and `Threads=8`.

## [1.3.4] - 2026-05-28

Patch release focused on completing Stockfish-compatible UCI ponder behavior
and Lazy SMP threaded search semantics.

### Fixed

- Fixed `ponderhit` handling so converting a ponder search to a normal search
  preserves elapsed thinking time instead of restarting the search clock. This
  matches Stockfish-style behavior and prevents tournament GUIs from allowing a
  ponder hit to consume an extra full move budget.
- Fixed threaded search accounting so UCI `nodes`, `tbhits`, stop, quit, and
  ponderhit state are shared across the main and helper threads rather than
  being reported or limited from the main thread alone.

### Changed

- Enabled threaded search for node-limited and MultiPV searches, with helpers
  using the same root move set and aggregate stop condition as the main thread.
- Reworked parallel best-result selection to use weighted votes from helper
  searches, with decisive scores, PV availability, depth, score, and the main
  thread used as tie-breakers.

### Added

- Added interactive UCI process regression tests for completed `go ponder`
  searches waiting for `ponderhit` or `stop` before emitting `bestmove`.
- Added regression coverage verifying that `ponderhit` after an already-spent
  `movetime` returns `bestmove` promptly rather than restarting the timer.
- Added UCI process regression tests for threaded `go nodes`, threaded
  `go infinite` plus `stop`, and threaded `go ponder` plus `ponderhit`.
- Added engine-level coverage for threaded aggregate node limits and threaded
  MultiPV legal root results.
- Documented release-preparation checks for `1.3.4`, including the release
  test suite and a short cutechess regression against `v1.3.3` at one and
  eight search threads.

## [1.3.3] - 2026-05-28

Patch release focused on UCI tournament compatibility and release-process
coverage.

### Added

- Added UCI `go perft N` support so GUI and tournament tools can ask the engine
  to count legal leaf nodes from the current position without starting a normal
  search.
- Added the advertised UCI `Ponder` option. This lets frontends discover that
  Lynx supports ponder-mode command flow even though the option itself is only a
  protocol setting.
- Added process-level UCI regression tests that launch the built engine binary
  and verify option advertisement, diagnostics, `go perft`, uppercase move
  input, and critical invalid-position handling.

### Changed

- Changed invalid `position` failures to report a clear critical UCI error and
  exit instead of continuing from the previous board state.
- Changed unknown UCI commands and unknown options to print explicit diagnostic
  messages.
- Changed empty `go` commands to run as an unbounded search until a limit or
  control command stops the search, instead of using a hidden shallow default.
- Changed UCI coordinate move parsing to accept uppercase square text, while
  still normalizing internally to the canonical lowercase move form.
- Changed `setoption` handling to wait for any active search to finish before
  applying engine configuration.

### Fixed

- Fixed compatibility with tournament managers that emit the common
  non-standard fullmove number `0` by normalizing it to fullmove `1` during FEN
  parsing.
- Fixed root draw positions, including fifty-move claim and dead-material
  positions, so the engine still returns a legal move when legal moves exist.
  `bestmove 0000` is now reserved for positions with no legal moves.

## [1.3.2] - 2026-05-28

### Added

- Added a true threshold `see_ge` implementation so search pruning and move
  ordering can test exchange safety without always computing full SEE.
- Added transposition-table prefetching before child probes in main search,
  quiescence search, null-move search, ProbCut, and MultiPV root searches.
- Added root-effort-aware soft time scaling so unstable or low-confidence root
  searches can spend more of the available soft budget.
- Added regression coverage for promotion SEE thresholds, staged bad-capture
  ordering, mixed valid/invalid `searchmoves`, and current-generation
  `hashfull` accounting.

### Changed

- Changed staged move picking to search good captures first, generate quiets
  lazily, and delay losing captures until after quiet moves.
- Changed tactical capture scoring and pruning to use threshold SEE in hot
  paths, reducing full-SEE work during search.
- Made late-move reductions cut-node aware and slightly more sensitive to
  losing captures and strong quiet history.
- Made quiescence pruning use dynamic threshold SEE and late losing-capture
  filtering.
- Changed `hashfull` reporting to count only entries from the current TT
  generation for both local and shared tables.

### Fixed

- Fixed SEE accounting for promotion captures by using the promoted piece as
  the next attacker in the exchange sequence.
- Fixed invalid `go searchmoves` input that matches no legal root move to fall
  back to the full legal root move list instead of returning a spurious draw.
- Fixed excluded-move searches so the excluded move is not counted as a legal
  searched move.

## [1.3.1] - 2026-05-28

### Added

- Added stricter FEN validation for complete ranks, legal pawn ranks, one king
  per side, adjacent kings, side-not-to-move check legality, castling-right
  consistency, and valid move counters.
- Added canonical en passant hashing so EP squares are kept in the position key
  only when a legal en passant capture exists.
- Added continuation correction history to the handcrafted-evaluation
  correction path.
- Added UCI `go searchmoves` root filtering and `go mate` depth handling.
- Added regression coverage for invalid FEN rejection, canonical EP handling,
  rule-50-aware TT mate recovery, insufficient-material search draws,
  `searchmoves`, and `mate` parsing.

### Changed

- Made transposition-table mate score recovery aware of the current rule-50
  counter to avoid reusing forced-mate scores past the 50-move horizon.
- Changed more search cutoffs to preserve fail-soft scores in TT storage and
  return values.
- Added staged main-search move picking so captures can be searched before
  quiet moves are generated at non-root non-check nodes.
- Reworked soft/hard clock allocation with a move-count horizon, explicit
  `movestogo` support, increment handling, and move-overhead reserve.
- Extended quiescence search depth and avoided static-evaluation fallback while
  still in check.
- Made quiet halfmove-clock increments saturating for robustness on high-clock
  positions.

### Fixed

- Fixed search scoring for insufficient-material positions so dead draws return
  an immediate draw result instead of non-zero material/PST scores.
- Fixed a crash path where illegal FENs with the side not to move already in
  check could reach move generation.

## [1.3.0] - 2026-05-27

### Added

- Added incremental pawn, minor-piece, and color-specific non-pawn structure
  keys for more precise pawn keys and richer correction history indexing.
- Added low-ply quiet history and pawn-structure-indexed quiet history to
  improve move ordering in the main search.
- Added additional continuation-history channels at wider ply distances.
- Added high-depth null-move verification to reduce tactical risk from
  aggressive null-move cutoffs.
- Added capture futility pruning in quiescence search.
- Added fifty-move-rule dampening to the handcrafted evaluation.
- Added regression coverage for pawn-key color/structure collisions and
  evaluation scaling near a fifty-move-rule draw.

### Changed

- Reworked static-evaluation handling so transposition-table entries store raw
  static evals and apply the current correction history on probe.
- Improved pruning selectivity with TT-assisted pruning evals, improving-aware
  futility margins, and refined shallow razoring/null-move conditions.
- Improved tactical move ordering with SEE-aware capture scores and capture
  history while ordering losing captures behind quiet candidates.
- Improved root time usage by extending past the soft limit after material
  score drops and by stopping early in forced single-legal-move positions.
- Improved clock allocation for explicit `movestogo` and sudden-death controls
  with more conservative hard limits and increment handling.

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
- Improved Syzygy UCI defaults and practical root tablebase usage for common
  GUI and tournament-manager setups.
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
  `ponderhit`, `stop`, or `quit`, matching the UCI expectation that a completed
  ponder result is retained but not reported early.
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
