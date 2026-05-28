use std::io::Write;
use std::process::{Command, Output, Stdio};

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
