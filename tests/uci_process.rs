use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

fn run_lynx(input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lynx"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lynx binary should start");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("test input should be written");
    drop(child.stdin.take());

    child.wait_with_output().expect("lynx should exit")
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_lynx"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("lynx binary should start");

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
        let deadline = Instant::now() + timeout;
        let mut seen = Vec::new();
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                panic!("timed out waiting for `{needle}`; seen: {seen:?}");
            }
            match self.stdout_rx.recv_timeout(remaining) {
                Ok(line) if line.contains(needle) => return line,
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
                    panic!("lynx stdout closed while checking for absence of `{needle}`");
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
                .expect("lynx process should be waitable")
                .success(),
            "lynx should exit successfully"
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
    let output = run_lynx("uci\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("id name Lynx"));
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
fn unknown_command_and_option_print_explicit_diagnostics() {
    let output = run_lynx("setoption name Not A Real Option value 1\nunknownthing\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("No such option: Not A Real Option"));
    assert!(out.contains("Unknown command: 'unknownthing'. Type help for more information."));
}

#[test]
fn go_perft_runs_synchronously_before_following_quit() {
    let output = run_lynx("go perft 1\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("Nodes searched: 20"));
    assert!(!out.contains("bestmove"), "{out}");
}

#[test]
fn position_accepts_uppercase_move_text_before_perft() {
    let output = run_lynx("position startpos moves E2E4\ngo perft 2\nquit\n");

    assert!(output.status.success(), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains("Nodes searched: 600"), "{out}");
}

#[test]
fn invalid_position_fen_is_a_critical_exit() {
    let output = run_lynx("position fen invalid\n");

    assert_eq!(output.status.code(), Some(1), "status: {:?}", output.status);
    let out = stdout(&output);
    assert!(out.contains(
        "info string CRITICAL ERROR: Command `position fen invalid` failed. Reason: Invalid FEN."
    ));
}
