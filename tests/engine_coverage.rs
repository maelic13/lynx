use lynx::board::{Board, Color, GameResult, Move, Piece, Square};
use lynx::engine_command::EngineCommand;
use lynx::eval::{Evaluator, MATE_SCORE, piece_value};
use lynx::search::{SearchEvent, SearchExit, Searcher};
use lynx::search_options::SearchOptions;
use lynx::syzygy;
use lynx::tt::{Bound, TranspositionTable, score_from_tt, score_to_tt};

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| (*part).to_string()).collect()
}

fn piece_at(board: &Board, square: Square) -> Option<(Color, Piece)> {
    board.piece_at(square)
}

#[test]
fn search_options_parse_startpos_moves_and_go_limits() {
    let mut options = SearchOptions::default();

    options
        .set_position(&args(&["startpos", "moves", "e2e4", "e7e5", "g1f3"]))
        .expect("valid startpos moves");

    assert_eq!(options.position.board.side_to_move(), Color::Black);
    assert_eq!(
        piece_at(&options.position.board, Square::E4),
        Some((Color::White, Piece::Pawn))
    );
    assert_eq!(
        piece_at(&options.position.board, Square::E5),
        Some((Color::Black, Piece::Pawn))
    );
    assert_eq!(
        piece_at(&options.position.board, Square::F3),
        Some((Color::White, Piece::Knight))
    );

    options.set_search_parameters(&args(&[
        "wtime",
        "10000",
        "btime",
        "9000",
        "winc",
        "100",
        "binc",
        "200",
        "movestogo",
        "20",
        "nodes",
        "12345",
        "ponder",
        "depth",
        "7",
    ]));

    assert_eq!(options.limits.white_time, 10_000);
    assert_eq!(options.limits.black_time, 9_000);
    assert_eq!(options.limits.white_increment, 100);
    assert_eq!(options.limits.black_increment, 200);
    assert_eq!(options.limits.movestogo, 20);
    assert_eq!(options.limits.nodes, 12_345);
    assert_eq!(options.limits.depth, 7.0);
    assert!(options.limits.search_moves.is_empty());
    assert!(options.limits.ponder);
    assert!(!options.limits.infinite);

    options.set_search_parameters(&args(&["ponder", "infinite"]));
    assert!(options.limits.ponder);
    assert!(options.limits.infinite);
    assert_eq!(options.limits.depth, f64::INFINITY);

    options.set_search_parameters(&args(&["depth", "3"]));
    assert!(!options.limits.ponder);
    assert!(!options.limits.infinite);
    assert_eq!(options.limits.depth, 3.0);

    options.set_search_parameters(&args(&["mate", "2", "searchmoves", "e2e4", "g1f3"]));
    assert_eq!(options.limits.depth, 3.0);
    assert_eq!(options.limits.search_moves.len(), 2);
}

#[test]
fn search_options_accept_uppercase_uci_move_text() {
    let mut options = SearchOptions::default();

    options
        .set_position(&args(&["startpos", "moves", "E2E4"]))
        .expect("uppercase UCI move text should be normalized");

    assert_eq!(options.position.board.side_to_move(), Color::Black);
    assert_eq!(
        piece_at(&options.position.board, Square::E4),
        Some((Color::White, Piece::Pawn))
    );
}

#[test]
fn search_options_default_go_and_invalid_limits_are_bounded() {
    let mut options = SearchOptions::default();

    options.set_search_parameters(&[]);

    assert_eq!(options.limits.depth, f64::INFINITY);
    assert_eq!(options.limits.nodes, 0);
    assert_eq!(options.limits.perft, 0);
    assert!(!options.limits.infinite);
    assert!(!options.limits.ponder);

    options.set_search_parameters(&args(&[
        "depth",
        "not-a-number",
        "nodes",
        "bad",
        "movetime",
        "none",
        "wtime",
        "missing",
        "btime",
        "also-bad",
        "movestogo",
        "oops",
    ]));

    assert_eq!(options.limits.depth, 2.0);
    assert_eq!(options.limits.nodes, 0);
    assert_eq!(options.limits.perft, 0);
    assert_eq!(options.limits.move_time, 0);
    assert_eq!(options.limits.white_time, 0);
    assert_eq!(options.limits.black_time, 0);
    assert_eq!(options.limits.movestogo, 0);
}

#[test]
fn search_options_parse_uci_go_perft() {
    let mut options = SearchOptions::default();

    options.set_search_parameters(&args(&["perft", "3"]));

    assert_eq!(options.limits.perft, 3);
    assert_eq!(options.limits.depth, f64::INFINITY);
}

#[test]
fn search_options_setoption_and_reset_cover_engine_configuration() {
    let mut options = SearchOptions::default();

    assert!(options.set_option(&args(&["name", "Hash", "value", "256"])));
    assert!(options.set_option(&args(&["name", "Move", "Overhead", "value", "25"])));
    assert!(options.set_option(&args(&["name", "Threads", "value", "99"])));
    assert!(options.set_option(&args(&["name", "MultiPV", "value", "3"])));
    assert!(options.set_option(&args(&["name", "Ponder", "value", "true"])));
    assert!(options.set_option(&args(&["name", "SyzygyPath", "value", "C:\\TB\\WDL"])));
    assert!(options.set_option(&args(&["name", "SyzygyProbeDepth", "value", "6"])));
    assert!(options.set_option(&args(&["name", "SyzygyProbeLimit", "value", "5"])));
    assert!(options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "false"])));
    assert!(options.set_option(&args(&["name", "Clear", "Hash"])));

    assert_eq!(options.engine.hash_mb, 256);
    assert_eq!(options.engine.move_overhead, 25.0);
    assert_eq!(options.engine.threads, 99);
    assert_eq!(options.engine.multi_pv, 3);
    assert!(options.engine.ponder);
    assert_eq!(options.engine.syzygy.path, "C:\\TB\\WDL");
    assert_eq!(options.engine.syzygy.probe_depth, 6);
    assert_eq!(options.engine.syzygy.probe_limit, 5);
    assert!(!options.engine.syzygy.fifty_move_rule);
    assert!(options.engine.clear_hash);

    assert!(options.set_option(&args(&["name", "Threads", "value", "9999"])));
    assert_eq!(options.engine.threads, 1024);

    options.set_search_parameters(&args(&[
        "depth", "4", "nodes", "99", "movetime", "500", "ponder",
    ]));
    options.reset();

    assert_eq!(options.limits.depth, f64::INFINITY);
    assert_eq!(options.limits.nodes, 0);
    assert_eq!(options.limits.move_time, 0);
    assert!(options.limits.search_moves.is_empty());
    assert!(!options.limits.ponder);
    assert!(!options.limits.infinite);
    assert_eq!(options.engine.hash_mb, 256);
    assert_eq!(options.engine.move_overhead, 25.0);
    assert_eq!(options.engine.syzygy.path, "C:\\TB\\WDL");
    assert_eq!(options.engine.multi_pv, 3);

    let names = SearchOptions::get_uci_options().join("\n");
    assert!(names.contains("option name Hash"));
    assert!(names.contains("option name Move Overhead"));
    assert!(names.contains("option name Threads type spin default 1 min 1 max 1024"));
    assert!(names.contains("option name MultiPV type spin default 1 min 1 max 256"));
    assert!(names.contains("option name Clear Hash"));
    assert!(names.contains("option name Ponder type check default false"));
    assert!(names.contains("option name SyzygyPath type string default <empty>"));
    assert!(names.contains("option name SyzygyProbeDepth type spin default 1 min 1 max 100"));
    assert!(names.contains("option name SyzygyProbeLimit"));
    assert!(names.contains("option name Syzygy50MoveRule"));
}

#[test]
fn search_options_invalid_setoption_values_preserve_previous_values() {
    let mut options = SearchOptions::default();

    options.set_option(&args(&["name", "Hash", "value", "128"]));
    options.set_option(&args(&["name", "Move", "Overhead", "value", "35"]));
    options.set_option(&args(&["name", "Threads", "value", "4"]));
    options.set_option(&args(&["name", "MultiPV", "value", "2"]));
    options.set_option(&args(&["name", "SyzygyProbeDepth", "value", "8"]));
    options.set_option(&args(&["name", "SyzygyProbeLimit", "value", "5"]));
    options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "false"]));

    assert!(options.set_option(&args(&["name", "Hash", "value", "bad"])));
    assert!(options.set_option(&args(&["name", "Move", "Overhead", "value", "nan"])));
    assert!(options.set_option(&args(&["name", "Move", "Overhead", "value", "5001"])));
    assert!(options.set_option(&args(&["name", "Threads", "value", "bad"])));
    assert!(options.set_option(&args(&["name", "MultiPV", "value", "bad"])));
    assert!(options.set_option(&args(&["name", "SyzygyProbeDepth", "value", "bad"])));
    assert!(options.set_option(&args(&["name", "SyzygyProbeLimit", "value", "bad"])));
    assert!(options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "maybe"])));
    assert!(!options.set_option(&args(&["name", "Unknown", "Option", "value", "1"])));

    assert_eq!(options.engine.hash_mb, 128);
    assert_eq!(options.engine.move_overhead, 35.0);
    assert_eq!(options.engine.threads, 4);
    assert_eq!(options.engine.multi_pv, 2);
    assert_eq!(options.engine.syzygy.probe_depth, 8);
    assert_eq!(options.engine.syzygy.probe_limit, 5);
    assert!(!options.engine.syzygy.fifty_move_rule);
}

#[test]
fn search_options_clamp_syzygy_values_and_preserve_raw_path() {
    let mut options = SearchOptions::default();
    assert_eq!(options.engine.syzygy.probe_depth, 1);

    options.set_option(&args(&[
        "name",
        "SyzygyPath",
        "value",
        "C:\\TB",
        "Mixed Case",
    ]));
    options.set_option(&args(&["name", "SyzygyProbeDepth", "value", "0"]));
    options.set_option(&args(&["name", "SyzygyProbeLimit", "value", "99"]));
    options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "false"]));

    assert_eq!(options.engine.syzygy.path, "C:\\TB Mixed Case");
    assert_eq!(options.engine.syzygy.probe_depth, 1);
    assert_eq!(options.engine.syzygy.probe_limit, 7);
    assert!(!options.engine.syzygy.fifty_move_rule);

    options.set_option(&args(&["name", "SyzygyProbeDepth", "value", "250"]));
    options.set_option(&args(&["name", "SyzygyProbeLimit", "value", "0"]));
    options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "maybe"]));
    options.set_option(&args(&["name", "MultiPV", "value", "999"]));

    assert_eq!(options.engine.syzygy.probe_depth, 100);
    assert_eq!(options.engine.syzygy.probe_limit, 0);
    assert_eq!(options.engine.multi_pv, 256);
    assert!(
        !options.engine.syzygy.fifty_move_rule,
        "invalid boolean value must leave the previous setting unchanged"
    );

    options.set_option(&args(&["name", "Syzygy50MoveRule", "value", "true"]));
    assert!(options.engine.syzygy.fifty_move_rule);
}

#[test]
fn search_options_reject_illegal_position_move_without_losing_current_board() {
    let mut options = SearchOptions::default();

    options
        .set_position(&args(&["startpos", "moves", "e2e4"]))
        .expect("valid startpos move");
    let expected = options.position.board.clone();

    let err = options
        .set_position(&args(&["startpos", "moves", "e2e5"]))
        .expect_err("illegal position move should be reported");

    assert_eq!(err, "Illegal move: e2e5");
    assert_eq!(options.position.board.hash, expected.hash);
    assert_eq!(options.position.board.to_fen(), expected.to_fen());
}

#[test]
fn search_options_accept_little_blitzer_fullmove_zero_fen() {
    let mut options = SearchOptions::default();

    options
        .set_position(&args(&[
            "fen",
            "r1bqkb1r/pppn1ppp/3p1n2/4p1B1/3PP3/2N5/PPP2PPP/R2QKBNR",
            "w",
            "KQkq",
            "e6",
            "0",
            "0",
            "moves",
            "d4d5",
        ]))
        .expect("compatible fullmove-zero FEN");

    assert_eq!(options.position.board.side_to_move(), Color::Black);
    assert_eq!(
        options.position.board.piece_at(Square::D5),
        Some((Color::White, Piece::Pawn))
    );
    assert_eq!(options.position.board.fullmove, 1);
}

#[test]
fn engine_command_constructors_set_expected_flags() {
    let options = SearchOptions::default();

    let go = EngineCommand::go(options, 11);
    assert!(!go.stop);
    assert!(!go.quit);
    assert!(go.bench_depth.is_none());
    assert!(go.configure.is_none());
    assert!(!go.new_game);
    assert!(!go.ponderhit);
    assert_eq!(go.epoch, 11);

    let stop = EngineCommand::stop(12);
    assert!(stop.stop);
    assert!(!stop.quit);
    assert_eq!(stop.epoch, 12);

    let quit = EngineCommand::quit(13);
    assert!(quit.quit);
    assert!(quit.stop);
    assert_eq!(quit.epoch, 13);

    let bench = EngineCommand::bench(7, SearchOptions::default(), 14);
    assert_eq!(bench.bench_depth, Some(7));
    assert_eq!(bench.epoch, 14);

    let configure = EngineCommand::configure(SearchOptions::default());
    assert!(configure.configure.is_some());

    let new_game = EngineCommand::new_game();
    assert!(new_game.new_game);

    let ponderhit = EngineCommand::ponderhit();
    assert!(ponderhit.ponderhit);
}

#[test]
fn transposition_table_store_probe_replace_clear_and_mate_scores() {
    let mut table = TranspositionTable::new(1);
    let key = 0x1234_0000_0000_0000;
    let best = Move::from_uci("e2e4").expect("valid UCI move");

    table.store(key, 5, 123, Bound::Exact, best, 0, 42);
    let entry = table.probe(key).expect("entry must be stored");
    assert_eq!(entry.score, 123);
    assert_eq!(entry.static_eval, 42);
    assert_eq!(entry.depth, 5);
    assert_eq!(entry.bound(), Some(Bound::Exact));
    assert_eq!(entry.best_move(), Some(best));

    table.store(0x5678_0000_0000_0001, 1, 1, Bound::Exact, best, 0, 0);
    assert!(table.hashfull() > 0);

    assert!(!table.resize(usize::MAX));
    assert!(
        table.probe(key).is_some(),
        "failed resize must keep the current table"
    );

    table.make_shared();
    assert!(
        table.probe(key).is_none(),
        "shared TT should not import local key16-only entries"
    );
    table.store(key, 5, 123, Bound::Exact, best, 0, 42);
    let shared_entry = table
        .probe(key)
        .expect("entry must be stored in shared table");
    assert_eq!(shared_entry.score, 123);
    assert_eq!(shared_entry.bound(), Some(Bound::Exact));
    assert_eq!(shared_entry.best_move(), Some(best));
    assert!(
        table.probe(key ^ 0x0000_8000_0000_0000).is_none(),
        "shared TT must validate the full key"
    );

    table.store(key, 4, 90, Bound::Upper, Move::NULL, 0, 11);
    let replaced = table.probe(key).expect("entry must remain present");
    assert_eq!(replaced.bound(), Some(Bound::Upper));
    assert_eq!(replaced.best_move(), Some(best));

    let mate_in_three = MATE_SCORE - 3;
    let tt_score = score_to_tt(mate_in_three, 7);
    assert_eq!(score_from_tt(tt_score, 7, 0), mate_in_three);
    let mated_in_three = -MATE_SCORE + 3;
    let tt_score = score_to_tt(mated_in_three, 7);
    assert_eq!(score_from_tt(tt_score, 7, 0), mated_in_three);
    assert!(
        score_from_tt(score_to_tt(MATE_SCORE - 12, 0), 0, 95) < MATE_SCORE - 128,
        "mate scores past the 50-move horizon must not be reused as forced mates"
    );

    table.clear();
    assert!(table.probe(key).is_none());
    assert_eq!(table.hashfull(), 0);
}

#[test]
fn transposition_table_hashfull_counts_only_current_generation_entries() {
    let best = Move::from_uci("e2e4").expect("valid UCI move");
    let key = 0xCAFE_0000_0000_0001;
    let second_key = 0xCAFE_0000_0000_0002;
    let fresh_key = 0xBABE_0000_0000_0003;
    let second_fresh_key = 0xBABE_0000_0000_0004;

    let mut table = TranspositionTable::new(1);
    table.store(key, 6, 12, Bound::Exact, best, 0, 34);
    table.store(second_key, 5, 20, Bound::Upper, best, 0, 10);
    table.prefetch(key);
    assert!(table.hashfull() > 0);

    table.new_search();
    assert_eq!(
        table.hashfull(),
        0,
        "hashfull should ignore entries from older TT generations"
    );
    assert!(
        table.probe(key).is_some(),
        "stale hashfull accounting must not make entries unprobeable"
    );

    table.store(fresh_key, 4, -8, Bound::Lower, best, 0, -10);
    table.store(second_fresh_key, 3, -12, Bound::Exact, best, 0, -3);
    assert!(table.hashfull() > 0);

    let mut shared = TranspositionTable::new(1);
    shared.make_shared();
    shared.store(key, 5, 99, Bound::Exact, best, 0, 11);
    shared.store(second_key, 4, 88, Bound::Lower, best, 0, 22);
    shared.prefetch(key);
    assert!(shared.hashfull() > 0);
    shared.new_search();
    assert_eq!(shared.hashfull(), 0);
    assert!(shared.probe(key).is_some());
}

#[test]
fn evaluator_scores_material_from_side_to_move_perspective() {
    let mut evaluator = Evaluator::default();
    let white_to_move = Board::from_fen("4k3/8/8/8/8/8/8/Q3K3 w - - 0 1").expect("valid FEN");
    let black_to_move = Board::from_fen("4k3/8/8/8/8/8/8/Q3K3 b - - 0 1").expect("valid FEN");

    assert!(evaluator.evaluate(&white_to_move) > piece_value(Piece::Queen) - 100);
    assert!(evaluator.evaluate(&black_to_move) < -piece_value(Piece::Queen) + 100);
}

#[test]
fn evaluator_dampens_static_advantage_near_fifty_move_draw() {
    let mut evaluator = Evaluator::default();
    let fresh = Board::from_fen("4k3/8/8/8/8/8/8/Q3K3 w - - 0 1").expect("valid FEN");
    let stale = Board::from_fen("4k3/8/8/8/8/8/8/Q3K3 w - - 90 46").expect("valid FEN");

    assert!(evaluator.evaluate(&stale).abs() < evaluator.evaluate(&fresh).abs());
}

#[test]
fn syzygy_disabled_path_leaves_tablebase_probes_unavailable() {
    let board = Board::from_fen("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");

    assert_eq!(syzygy::initialize(""), 0);
    assert_eq!(syzygy::largest(), 0);
    assert!(syzygy::probe_wdl(&board, true).is_none());
    assert!(syzygy::probe_root(&board, true).is_none());
}

#[test]
fn evaluator_rewards_advanced_protected_passers_over_back_rank_pawns() {
    let mut evaluator = Evaluator::default();
    let advanced_connected =
        Board::from_fen("4k3/8/4P3/3P4/8/8/8/4K3 w - - 0 1").expect("valid FEN");
    let undeveloped = Board::from_fen("4k3/8/8/8/8/3P4/4P3/4K3 w - - 0 1").expect("valid FEN");

    assert!(evaluator.evaluate(&advanced_connected) > evaluator.evaluate(&undeveloped));
}

#[test]
fn evaluator_scales_known_drawish_minor_endgames() {
    let mut evaluator = Evaluator::default();
    let two_knights_vs_bare_king =
        Board::from_fen("7k/8/8/8/8/8/8/NN2K3 w - - 0 1").expect("valid FEN");

    assert_eq!(evaluator.evaluate(&two_knights_vs_bare_king), 0);
}

#[test]
fn evaluator_reports_terminal_results_with_distance_to_mate() {
    let evaluator = Evaluator::default();

    assert_eq!(
        evaluator.evaluate_result(GameResult::WhiteCheckmates, Color::White, 3),
        MATE_SCORE - 3
    );
    assert_eq!(
        evaluator.evaluate_result(GameResult::WhiteCheckmates, Color::Black, 3),
        -MATE_SCORE + 3
    );
    assert_eq!(
        evaluator.evaluate_result(GameResult::BlackCheckmates, Color::Black, 11),
        MATE_SCORE - 11
    );
    assert_eq!(
        evaluator.evaluate_result(GameResult::Draw, Color::White, 99),
        0
    );
}

#[test]
fn search_returns_null_move_for_stalemate() {
    let board = Board::from_fen("4k3/4P3/4K3/8/8/8/8/8 b - - 0 1").expect("valid FEN");
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 4.0;

    let result = searcher.search(board, &options, false, || SearchEvent::None);

    assert_eq!(result.bestmove, Move::NULL);
    assert_eq!(result.pondermove, Move::NULL);
    assert_eq!(result.depth, 0);
    assert_eq!(result.score, 0);
    assert_eq!(result.tb_hits, 0);
    assert_eq!(result.exit, SearchExit::Stop);
}

#[test]
fn search_returns_legal_root_move_in_drawn_material_positions() {
    for fen in [
        "8/8/8/8/8/8/4K3/6k1 w - - 0 1",
        "7k/8/8/8/8/8/4KN2/8 w - - 0 1",
        "7k/8/8/8/8/8/4KB2/8 w - - 0 1",
    ] {
        let board = Board::from_fen(fen).expect("valid draw FEN");
        let legal_moves = board.generate_legal_movelist();
        let mut searcher = Searcher::default();
        let mut options = SearchOptions::default();
        options.limits.depth = 1.0;

        let result = searcher.search(board, &options, false, || SearchEvent::None);

        assert_eq!(result.score, 0, "{fen}");
        assert_ne!(result.bestmove, Move::NULL, "{fen}");
        assert!(
            legal_moves
                .iter()
                .any(|&legal_move| legal_move.same_uci_move(result.bestmove)),
            "{} must be legal for {fen}",
            result.bestmove
        );
    }
}

#[test]
fn search_returns_legal_move_in_root_fifty_move_claim_position() {
    let board =
        Board::from_fen("8/8/7k/8/1N6/1K6/4r3/8 w - - 100 1").expect("valid fifty-move claim FEN");
    let legal_moves = board.generate_legal_movelist();
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 1.0;

    let result = searcher.search(board, &options, false, || SearchEvent::None);

    assert_ne!(result.bestmove, Move::NULL);
    assert_eq!(result.score, 0);
    assert!(
        legal_moves
            .iter()
            .any(|&legal_move| legal_move.same_uci_move(result.bestmove)),
        "{} must be legal",
        result.bestmove
    );
}

#[test]
fn search_respects_searchmoves_root_filter() {
    let board = Board::default();
    let forced = board.parse_move("a2a3").expect("legal root move");
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.set_search_parameters(&args(&["depth", "2", "searchmoves", "a2a3"]));

    let result = searcher.search(board, &options, false, || SearchEvent::None);

    assert_eq!(result.bestmove, forced);
}

#[test]
fn search_uses_matching_searchmoves_when_some_requested_moves_are_illegal() {
    let board = Board::default();
    let forced = board.parse_move("a2a3").expect("legal root move");
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.set_search_parameters(&args(&["depth", "2", "searchmoves", "a7a6", "a2a3"]));

    let result = searcher.search(board, &options, false, || SearchEvent::None);

    assert_eq!(result.bestmove, forced);
}

#[test]
fn search_falls_back_when_searchmoves_match_no_root_move() {
    let board = Board::default();
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.set_search_parameters(&args(&["depth", "1", "searchmoves", "a7a6"]));

    let result = searcher.search(board, &options, false, || SearchEvent::None);

    assert_ne!(result.bestmove, Move::NULL);
    assert_eq!(result.depth, 1);
}

#[test]
fn search_respects_node_limit() {
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 99.0;
    options.limits.nodes = 512;

    let result = searcher.search(options.position.board.clone(), &options, false, || {
        SearchEvent::None
    });

    assert_eq!(result.exit, SearchExit::Stop);
    assert!(result.nodes >= 512, "nodes: {}", result.nodes);
    assert!(result.nodes <= 2_048, "nodes: {}", result.nodes);
    assert!(result.depth < 99);
}

#[test]
fn threaded_search_uses_aggregate_node_limit() {
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 99.0;
    options.limits.nodes = 512;
    options.engine.threads = 8;

    let result = searcher.search(options.position.board.clone(), &options, false, || {
        SearchEvent::None
    });

    assert_eq!(result.exit, SearchExit::Stop);
    assert!(result.nodes >= 512, "nodes: {}", result.nodes);
    assert!(
        result.nodes <= 2_048,
        "threaded node-limited search should not multiply the limit: {}",
        result.nodes
    );
    assert!(result.depth < 99);
}

#[test]
fn search_quit_event_exits_search() {
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 99.0;

    let mut polls = 0;
    let result = searcher.search(options.position.board.clone(), &options, false, || {
        polls += 1;
        SearchEvent::Quit
    });

    assert_eq!(result.exit, SearchExit::Quit);
    assert!(polls > 0);
    assert!(result.nodes >= 512, "nodes: {}", result.nodes);
    assert!(result.depth < 99);
}

#[test]
fn search_result_records_ponderhit_conversion() {
    let mut searcher = Searcher::default();
    let mut options = SearchOptions::default();
    options.limits.depth = 99.0;
    options.limits.nodes = 4_096;
    options.limits.ponder = true;

    let mut polls = 0;
    let result = searcher.search(options.position.board.clone(), &options, false, || {
        polls += 1;
        if polls == 1 {
            SearchEvent::PonderHit
        } else {
            SearchEvent::None
        }
    });

    assert_eq!(result.exit, SearchExit::Stop);
    assert!(result.ponderhit);
    assert!(polls > 0);
    assert!(result.nodes >= 4_096, "nodes: {}", result.nodes);
    assert!(result.depth < 99);
}
