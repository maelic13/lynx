# Lynx

Lynx is a UCI-compatible chess engine written in Rust.
The engine is intended for use from a chess GUI or engine-testing tool that
speaks the UCI protocol.

## Highlights

- Custom bitboard board representation with incremental make/unmake
- Legal move generation for all standard chess rules, including castling,
  en passant, promotions, repetition, the fifty-move rule, and insufficient
  material detection
- Strict FEN validation with canonical en passant hashing so positions without
  a legal en passant capture share the same transposition key
- Zobrist hashing, transposition table, and pawn evaluation cache
- Iterative deepening negamax/PVS search with aspiration windows
- Configurable Lazy SMP-style parallel search with persistent workers, shared
  stop/node/accounting state, weighted helper result selection, and a full-key
  validated shared transposition table through the UCI `Threads` option
- Basic UCI `MultiPV` support for analysis output
- Capture-focused quiescence search with delta pruning, capture futility,
  threshold SEE pruning, and bounded check evasions
- Null-move pruning with verification, ProbCut, singular extensions, futility
  pruning, late move pruning, and late move reductions
- Staged move picking with a validated TT move first, good captures before lazy
  quiet generation, and bad captures delayed until after quiet moves
- Move ordering using TT moves, threshold SEE, killers, countermoves, main
  history, eight-ply low-ply history, pawn history, capture history,
  continuation history, and a direct-check bonus for quiet checking moves
- Direct legal validation for raw UCI and TT-shaped moves, including
  canonicalized captures, castling, en passant, and promotions
- Multi-table and continuation correction history with handcrafted tapered
  evaluation and fifty-move-rule dampening
- Soft/hard time allocation with `movestogo`, increment, and move-overhead
  handling
- Optional Syzygy tablebase probing through the UCI `SyzygyPath`,
  `SyzygyProbeDepth`, `SyzygyProbeLimit`, and `Syzygy50MoveRule` options,
  with root DTZ ranking, WDL fallback, load summaries, and `tbhits` reporting
- Built-in `bench` UCI command for repeatable search benchmarks

## UCI Support

Supported commands include:

- `uci`
- `isready`
- `ucinewgame`
- `position startpos [moves ...]`
- `position fen <fen> [moves ...]`
- `go` with `depth`, `nodes`, `movetime`, `wtime`, `btime`, `winc`, `binc`,
  `movestogo`, `mate`, `searchmoves`, `ponder`, `perft`, and `infinite`
- `stop`
- `ponderhit`
- `quit`
- `bench [depth]`

Supported options:

- `Hash` default `64`
- `Clear Hash`
- `Ponder` default `false`
- `Move Overhead` default `10`
- `Threads` default `1`, min `1`, max `1024`
- `MultiPV` default `1`, min `1`, max `256`
- `SyzygyPath` default empty
- `SyzygyProbeDepth` default `1`, min `1`, max `100`
- `SyzygyProbeLimit` default `7`, min `0`, max `7`
- `Syzygy50MoveRule` default `true`

`SyzygyPath` may contain one or more Syzygy directories separated by the
platform path separator (`;` on Windows, `:` on Unix-like systems). When the
path is empty, tablebase probing is disabled. Lynx uses WDL probes inside the
search and DTZ-ranked root probing when DTZ tables are available. If root DTZ
probing is unavailable but WDL tables are present, Lynx falls back to WDL root
move filtering. Search info includes `tbhits` when tablebase probes are used.
Set `SyzygyProbeLimit` to `0` to disable probing without changing the path.

## Bench

Run the built-in benchmark from a UCI session:

```text
bench
bench 13
```

The bench command searches a fixed suite of positions and reports a repeatable
search fingerprint and speed data. It is useful for comparing local changes,
compiler settings, and machine performance.

The benchmark uses the current UCI options, including `Threads`, so a threaded
search benchmark can be run with:

```text
setoption name Threads value 8
bench
```

Run the board implementation benchmark with:

```bash
cargo bench --bench board
```

This benchmark measures legal move generation, direct legal move validation,
capture generation, make/unmake, check detection, SEE over captures,
game-simulation-style move generation, and start-position perft depth 4.

## Build From Source

Install Rust and Cargo, then build an optimized release binary:

```bash
cargo build --release
```

The executable is created at:

- `target/release/lynx`
- `target/release/lynx.exe` on Windows

Release builds use LTO and a single codegen unit for engine speed.
Local release builds also use `target-cpu=native`, so `cargo build --release`
optimizes Lynx for the CPU on the build machine.

For quick local testing:

```bash
cargo run --release
```

## Test

Run the release test suite:

```bash
cargo test --release
```

The suite covers:

- FEN parsing and round-tripping
- Strict FEN legality checks, castling-right validation, and en passant
  canonicalization
- Legal move generation and special moves
- Direct legal move validation for raw UCI/TT-shaped moves
- Perft reference positions
- Hashing and make/unmake correctness
- Incremental pawn, minor-piece, and non-pawn structure keys
- Draw and terminal-result handling
- Insufficient-material draw handling at search root and interior nodes
- Legal `bestmove` reporting from root draw positions where legal moves still
  exist
- Search limits, invalid limit parsing, and stop/quit behavior
- UCI `go searchmoves` root filtering and `go mate` depth conversion
- Time-management behavior for fast clocks, `movetime`, side-to-move clocks,
  explicit `movestogo`, and unbounded fixed-depth searches
- Single-thread determinism and thread-count reconfiguration
- Threaded search node-limit, MultiPV, stop, quit, UCI info accounting, and
  ponderhit behavior
- Threshold SEE behavior for captures and promotions
- Staged move-picker ordering for bad captures after quiet moves
- Direct-check quiet move-ordering bonus
- Eight-ply low-ply history scoring and update boundaries
- Syzygy option parsing, result decoding, root move conversion, root tablebase
  probing, tablebase path counting, and disabled-path probe behavior
- UCI command ordering, priority quit/stop handling, and stale-search
  cancellation
- UCI ponder and infinite-search `bestmove` release timing
- UCI PV replay legality for tournament-derived TT/hash-move regression
  positions at one and eight search threads
- FEN compatibility for tournament managers that emit non-standard fullmove `0`
- Quiet/capture move-generation partitioning
- Evaluation and transposition table behavior
- Current-generation `hashfull` accounting for local and shared transposition
  tables
- Fifty-move-rule evaluation dampening
- Rule-50-aware transposition-table mate score recovery
- TT-first move-picker behavior and TT-derived ponder fallback
- UCI command handling and invalid `setoption` preservation

## Use With A GUI

1. Build or download a Lynx executable.
2. Add it as a UCI engine in your chess GUI.
3. Configure `Hash` and `Move Overhead` as needed.
4. Start an engine game or analysis session.

Tested GUI families include Arena, ChessBase/Fritz, ChessOK Aquarium, and
Hiarcs Chess Explorer. Other UCI-compatible GUIs should also work.

## Releases

Current documented release: `1.4.1`.

`1.4.1` is the first public release in the 1.4 series and includes the
unreleased 1.4.0 TT/hash-move safety work.

- [Latest release](https://github.com/maelic13/lynx/releases/latest)
- [All releases](https://github.com/maelic13/lynx/releases)

Release-preparation checks for `1.4.1`:

```bash
cargo fmt --check
cargo check
cargo test
cargo test --release
```

The `1.4.1` work was also checked against the previous local baseline commit
`11a7a07` with short Cutechess head-to-head smoke matches at `Threads=1` and
`Threads=8`, plus local board and search benchmarks for quick speed sanity
checks.

Release assets may include standalone executables for Windows, Linux, and
Apple Silicon macOS. Intel macOS release assets are not published.
GitHub release binaries are built with explicit portable CPU targets instead of
`target-cpu=native`, so they can be shared safely. Local `cargo build --release`
builds continue to use `target-cpu=native` through `.cargo/config.toml`.

Use the most advanced binary your CPU supports:

| Asset suffix | Use when |
| --- | --- |
| `x86-64` | You need the most compatible Intel/AMD 64-bit build. |
| `avx2` | Your Intel/AMD CPU supports x86-64-v3/AVX2; this is the usual optimized x64 choice. |
| `avx512` | Your Intel/AMD CPU supports x86-64-v4/AVX-512. |
| `arm64` | You are on ARM64 Linux, Windows on ARM, or Apple Silicon macOS. |

If unsure, use the plain `x86-64` or `arm64` asset for your operating system.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).

## Acknowledgements

Lynx is an independent engine, but it benefits from the open chess-engine
community's published ideas, testing practices, and protocol conventions.
Special thanks to Stockfish and its team for the inspiration their work provides
to chess engine authors and testers.
