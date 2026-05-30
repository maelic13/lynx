use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use rarog::board::Board;

fn run_rarog(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rarog"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Rarog binary should start");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("test input should be written");
    drop(child.stdin.take());

    child.wait_with_output().expect("Rarog should exit")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

struct UciSession {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<String>,
}

impl UciSession {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_rarog"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("Rarog binary should start");

        let stdin = child.stdin.take().expect("stdin should be piped");
        let stdout = child.stdout.take().expect("stdout should be piped");
        let (stdout_tx, stdout_rx) = mpsc::channel();

        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if stdout_tx.send(line).is_err() {
                    break;
                }
            }
        });

        Self {
            child,
            stdin,
            stdout_rx,
        }
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").expect("command should be written");
        self.stdin.flush().expect("command should be flushed");
    }

    fn expect_line_containing(&self, needle: &str, timeout: Duration) -> String {
        self.collect_until_line_containing(needle, timeout)
            .pop()
            .expect("matching line should be present")
    }

    fn collect_until_line_containing(&self, needle: &str, timeout: Duration) -> Vec<String> {
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for `{needle}`; seen: {seen:?}");
            }
            match self.stdout_rx.recv_timeout(remaining) {
                Ok(line) if line.contains(needle) => {
                    seen.push(line);
                    return seen;
                }
                Ok(line) => seen.push(line),
                Err(err) => panic!("timed out waiting for `{needle}` ({err}); seen: {seen:?}"),
            }
        }
    }

    fn assert_no_line_containing(&self, needle: &str, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match self
                .stdout_rx
                .recv_timeout(remaining.min(Duration::from_millis(25)))
            {
                Ok(line) if line.contains(needle) => {
                    panic!("unexpected `{needle}` line before release command: {line}");
                }
                Ok(_) => {}
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    panic!("Rarog stdout closed while checking for absence of `{needle}`");
                }
            }
        }
    }

    fn quit(mut self) {
        self.send("quit");
        self.stdin.flush().expect("quit should be flushed");
        assert!(
            self.child
                .wait()
                .expect("Rarog process should be waitable")
                .success(),
            "Rarog should exit successfully"
        );
    }
}

impl Drop for UciSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn uci_advertises_ponder_and_core_options() {
    let output = run_rarog("uci\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("id name Rarog"));
    assert!(out.contains("option name Ponder type check default false"));
    assert!(out.contains("option name Hash type spin default 64 min 1 max 33554432"));
    assert!(out.contains("uciok"));
}

#[test]
fn completed_ponder_search_waits_for_ponderhit_before_bestmove() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("position startpos moves e2e4");
    session.send("go ponder depth 1");

    session.expect_line_containing("info depth 1", Duration::from_secs(2));
    session.assert_no_line_containing("bestmove", Duration::from_millis(200));

    session.send("ponderhit");
    session.expect_line_containing("bestmove", Duration::from_secs(2));
    session.quit();
}

#[test]
fn completed_ponder_search_waits_for_stop_before_bestmove() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("position startpos moves e2e4");
    session.send("go ponder depth 1");

    session.expect_line_containing("info depth 1", Duration::from_secs(2));
    session.assert_no_line_containing("bestmove", Duration::from_millis(200));

    session.send("stop");
    session.expect_line_containing("bestmove", Duration::from_secs(2));
    session.quit();
}

#[test]
fn ponderhit_after_spent_movetime_does_not_restart_search_clock() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("position startpos moves e2e4");
    session.send("go ponder movetime 1000");

    thread::sleep(Duration::from_millis(1300));
    session.send("ponderhit");

    session.expect_line_containing("bestmove", Duration::from_millis(750));
    session.quit();
}

#[test]
fn threaded_go_nodes_returns_bestmove_and_reports_nodes() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("setoption name Threads value 4");
    session.send("isready");
    session.expect_line_containing("readyok", Duration::from_secs(5));
    session.send("position startpos");
    session.send("go nodes 4096");

    let lines = session.collect_until_line_containing("bestmove", Duration::from_secs(5));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("info depth")
                && parse_uci_u64_field(line, "nodes").is_some()),
        "threaded node search should emit at least one info line with nodes: {lines:?}"
    );
    assert!(
        lines
            .last()
            .is_some_and(|line| line.starts_with("bestmove ")),
        "search should finish with bestmove: {lines:?}"
    );
    session.quit();
}

#[test]
fn threaded_infinite_search_stops_cleanly() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("setoption name Threads value 4");
    session.send("isready");
    session.expect_line_containing("readyok", Duration::from_secs(5));
    session.send("position startpos");
    session.send("go infinite");

    session.expect_line_containing("info depth 1", Duration::from_secs(5));
    session.send("stop");
    session.expect_line_containing("bestmove", Duration::from_secs(5));
    session.quit();
}

#[test]
fn threaded_ponderhit_after_spent_movetime_does_not_restart_search_clock() {
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));
    session.send("setoption name Threads value 4");
    session.send("isready");
    session.expect_line_containing("readyok", Duration::from_secs(5));
    session.send("position startpos moves e2e4");
    session.send("go ponder movetime 1000");

    thread::sleep(Duration::from_millis(1300));
    session.send("ponderhit");

    session.expect_line_containing("bestmove", Duration::from_millis(750));
    session.quit();
}

#[test]
fn emitted_pvs_are_legal_for_tournament_positions_with_threads() {
    let fens = [
        "2k5/pp3pp1/5n2/2P5/bPP2P2/P3K3/6Pp/3Q1B1R w - - 0 23",
        "2k5/pp3pp1/5n2/2P5/1PP2P2/P2BK1p1/2b3PP/7R w - - 2 24",
        "2k5/pp3pp1/5n2/2P5/1PP2P2/P2BK1p1/6PP/3b3R b - - 1 23",
    ];
    let mut session = UciSession::start();
    session.send("uci");
    session.expect_line_containing("uciok", Duration::from_secs(15));

    for threads in [1, 8] {
        session.send(&format!("setoption name Threads value {threads}"));
        session.send("isready");
        session.expect_line_containing("readyok", Duration::from_secs(5));

        for fen in fens {
            session.send(&format!("position fen {fen}"));
            session.send("go depth 4");

            let lines = session.collect_until_line_containing("bestmove", Duration::from_secs(10));
            assert_uci_pv_lines_are_legal(fen, &lines);
        }
    }

    session.quit();
}

#[test]
fn unknown_command_and_option_print_explicit_diagnostics() {
    let output = run_rarog("setoption name Not A Real Option value 1\nunknownthing\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("No such option: Not A Real Option"));
    assert!(out.contains("Unknown command: 'unknownthing'. Type help for more information."));
}

#[test]
fn go_perft_runs_synchronously_before_following_quit() {
    let output = run_rarog("go perft 1\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("Nodes searched: 20"));
    assert!(!out.contains("bestmove"), "{out}");
}

#[test]
fn position_accepts_uppercase_move_text_before_perft() {
    let output = run_rarog("position startpos moves E2E4\ngo perft 2\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("Nodes searched: 600"), "{out}");
}

#[test]
fn invalid_position_fen_is_a_critical_exit() {
    let output = run_rarog("position fen invalid\n");

    assert_eq!(output.status.code(), Some(1), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains(
        "info string CRITICAL ERROR: Command `position fen invalid` failed. Reason: Invalid FEN."
    ));
}

fn parse_uci_u64_field(line: &str, field: &str) -> Option<u64> {
    let mut parts = line.split_whitespace();
    while let Some(part) = parts.next() {
        if part == field {
            return parts.next().and_then(|value| value.parse().ok());
        }
    }
    None
}

fn assert_uci_pv_lines_are_legal(root_fen: &str, lines: &[String]) {
    let mut saw_pv = false;
    for line in lines {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        let Some(pv_index) = parts.iter().position(|part| *part == "pv") else {
            continue;
        };
        saw_pv = true;
        let mut board = Board::from_fen(root_fen).unwrap_or_else(|err| panic!("{root_fen}: {err}"));
        for mv_text in &parts[pv_index + 1..] {
            let mv = board.parse_move(mv_text).unwrap_or_else(|| {
                panic!(
                    "illegal PV move `{mv_text}` in `{}` from line `{line}`",
                    board.to_fen()
                )
            });
            board.make_move_unchecked(mv);
        }
    }
    assert!(saw_pv, "search should emit at least one PV line: {lines:?}");
}
