# Lynx

Lynx is a UCI-compatible chess engine written in Rust.
The engine is intended for use from a chess GUI or engine-testing tool that
speaks the UCI protocol.

## Highlights

- Custom bitboard board representation with incremental make/unmake
- Legal move generation for all standard chess rules, including castling,
  en passant, promotions, repetition, the fifty-move rule, and insufficient
  material detection
- Zobrist hashing, transposition table, and pawn evaluation cache
- Iterative deepening negamax/PVS search with aspiration windows
- Configurable Lazy SMP-style parallel search with persistent workers and a
  full-key validated shared transposition table through the UCI `Threads`
  option
- Basic UCI `MultiPV` support for analysis output
- Capture-focused quiescence search with delta pruning, SEE-based pruning, and
  bounded check evasions
- Null-move pruning, ProbCut, singular extensions, futility pruning, late move
  pruning, and late move reductions
- Move ordering using TT moves, SEE, killers, countermoves, main history,
  capture history, and continuation history
- Correction history and handcrafted tapered evaluation
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
  `movestogo`, `ponder`, and `infinite`
- `stop`
- `ponderhit`
- `quit`
- `bench [depth]`

Supported options:

- `Hash` default `64`
- `Clear Hash`
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

This benchmark measures legal move generation, capture generation, make/unmake,
check detection, SEE over captures, game-simulation-style move generation, and
start-position perft depth 4.

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
- Legal move generation and special moves
- Perft reference positions
- Hashing and make/unmake correctness
- Draw and terminal-result handling
- Search limits, invalid limit parsing, and stop/quit behavior
- Time-management behavior for fast clocks, `movetime`, side-to-move clocks,
  explicit `movestogo`, and unbounded fixed-depth searches
- Single-thread determinism and thread-count reconfiguration
- Threaded search node-limit handling
- Syzygy option parsing, result decoding, root move conversion, root tablebase
  probing, tablebase path counting, and disabled-path probe behavior
- UCI command ordering, priority quit/stop handling, and stale-search
  cancellation
- UCI ponder and infinite-search `bestmove` release timing
- Quiet/capture move-generation partitioning
- Evaluation and transposition table behavior
- UCI command handling and invalid `setoption` preservation

## Use With A GUI

1. Build or download a Lynx executable.
2. Add it as a UCI engine in your chess GUI.
3. Configure `Hash` and `Move Overhead` as needed.
4. Start an engine game or analysis session.

Tested GUI families include Arena, ChessBase/Fritz, ChessOK Aquarium, and
Hiarcs Chess Explorer. Other UCI-compatible GUIs should also work.

## Releases

Current documented release: `1.2.1`.

- [Latest release](https://github.com/maelic13/lynx/releases/latest)
- [All releases](https://github.com/maelic13/lynx/releases)

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
