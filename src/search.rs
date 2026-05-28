use std::sync::{
    Arc, LazyLock,
    atomic::{AtomicU8, Ordering},
    mpsc,
};
use std::time::Instant;

use crate::board::{Board, Color, GameResult, Move, Piece};
use crate::eval::{Evaluator, INF_SCORE, MATE_SCORE, VALUE_NONE, piece_value};
use crate::move_ordering::{
    BadCaptureList, CAP_HISTORY_MAX, CONT_SIZE, CORR_SIZE, HISTORY_MAX, LOW_PLY_HISTORY_SIZE,
    PAWN_HISTORY_SIZE, PIECE_TO_SIZE, ScoredMove, ScoredMoveList, cont_index,
    diversify_root_scores, history_bonus, pawn_history_index, pick_next, piece_to_index,
    update_hist_entry,
};
use crate::search_options::{EngineOptions, MAX_THREADS, SearchLimits, SearchOptions};
use crate::search_threads::{STOP_NONE, STOP_QUIT, STOP_SEARCH, WorkerJob, WorkerPool};
use crate::syzygy::{self, Wdl};
use crate::time_manager::{RuntimeLimits, compute_runtime_limits};
use crate::tt::{Bound, TranspositionTable, score_from_tt};

const MAX_DEPTH: usize = 100;
const MAX_PLY: usize = 128;
const MAX_QPLY: usize = 16;
const MIN_PARALLEL_DEPTH: usize = 4;
const TB_WIN_SCORE: i32 = MATE_SCORE - MAX_PLY as i32 * 2;
static LMR_TABLE: LazyLock<[[i32; 64]; 64]> = LazyLock::new(|| {
    let mut table = [[0; 64]; 64];
    for (depth, row) in table.iter_mut().enumerate().skip(1) {
        for (move_index, value) in row.iter_mut().enumerate().skip(1) {
            *value = (0.75 + (depth as f64).ln() * (move_index as f64).ln() / 2.25) as i32;
        }
    }
    table
});
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchEvent {
    None,
    Stop,
    Quit,
    PonderHit,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SearchExit {
    Stop,
    Quit,
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub bestmove: Move,
    pub pondermove: Move,
    pub score: i32,
    pub depth: usize,
    pub nodes: u64,
    pub tb_hits: u64,
    pub elapsed_ms: u128,
    pub exit: SearchExit,
    pub ponderhit: bool,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum TtWriteMode {
    Main,
    Helper,
}

struct RootLine {
    score: i32,
    pv: Vec<Move>,
}

enum MovePicker {
    Full {
        scored: ScoredMoveList,
        index: usize,
    },
    Staged {
        captures: ScoredMoveList,
        bad_captures: ScoredMoveList,
        quiets: Option<ScoredMoveList>,
        capture_index: usize,
        bad_capture_index: usize,
        quiet_index: usize,
        tt_move: Move,
        ply: usize,
    },
}

pub struct Searcher {
    tt: TranspositionTable,
    hash_mb: usize,
    worker_pool: WorkerPool,
    evaluator: Evaluator,
    nodes: u64,
    tb_hits: u64,
    seldepth: usize,
    stopped: bool,
    quit: bool,
    pondering: bool,
    ponderhit: bool,
    start: Instant,
    limits: RuntimeLimits,
    pv_table: [[Move; MAX_PLY]; MAX_PLY],
    pv_len: [usize; MAX_PLY],
    stack_moves: [Move; MAX_PLY],
    stack_pieces: [Piece; MAX_PLY],
    stack_static_eval: [i32; MAX_PLY],
    killers: [[Move; 2]; MAX_PLY],
    root_moves: Vec<Move>,
    main_history: Box<[[[i16; 64]; 64]; 2]>,
    cap_history: Box<[[[i16; 6]; 64]; 6]>,
    low_ply_history: Box<[[[i16; 64]; 64]; LOW_PLY_HISTORY_SIZE]>,
    pawn_history: Vec<i16>,
    cont_history_1: Vec<i16>,
    cont_history_2: Vec<i16>,
    cont_history_4: Vec<i16>,
    cont_history_6: Vec<i16>,
    correction_history: Box<[[i16; CORR_SIZE]; 2]>,
    minor_correction_history: Box<[[i16; CORR_SIZE]; 2]>,
    non_pawn_correction_history: Box<[[[i16; CORR_SIZE]; 2]; 2]>,
    continuation_correction_history: Vec<i16>,
    countermove: Box<[[Move; 64]; 64]>,
    root_move_offset: usize,
    tt_write_mode: TtWriteMode,
    multi_pv: usize,
    syzygy_probe_depth: i32,
    syzygy_probe_limit: usize,
    syzygy_50_move_rule: bool,
    syzygy_largest: usize,
    root_iteration_nodes: u64,
    root_best_nodes: u64,
    root_best_effort: f64,
}

impl Default for Searcher {
    fn default() -> Self {
        Self {
            tt: TranspositionTable::default(),
            hash_mb: 64,
            worker_pool: WorkerPool::default(),
            evaluator: Evaluator::default(),
            nodes: 0,
            tb_hits: 0,
            seldepth: 0,
            stopped: false,
            quit: false,
            pondering: false,
            ponderhit: false,
            start: Instant::now(),
            limits: RuntimeLimits {
                depth: MAX_DEPTH,
                nodes: 0,
                soft_ms: f64::INFINITY,
                hard_ms: f64::INFINITY,
            },
            pv_table: [[Move::NULL; MAX_PLY]; MAX_PLY],
            pv_len: [0; MAX_PLY],
            stack_moves: [Move::NULL; MAX_PLY],
            stack_pieces: [Piece::Pawn; MAX_PLY],
            stack_static_eval: [VALUE_NONE; MAX_PLY],
            killers: [[Move::NULL; 2]; MAX_PLY],
            root_moves: Vec::new(),
            main_history: Box::new([[[0; 64]; 64]; 2]),
            cap_history: Box::new([[[0; 6]; 64]; 6]),
            low_ply_history: Box::new([[[0; 64]; 64]; LOW_PLY_HISTORY_SIZE]),
            pawn_history: vec![0; PAWN_HISTORY_SIZE * PIECE_TO_SIZE],
            cont_history_1: vec![0; CONT_SIZE],
            cont_history_2: vec![0; CONT_SIZE],
            cont_history_4: vec![0; CONT_SIZE],
            cont_history_6: vec![0; CONT_SIZE],
            correction_history: Box::new([[0; CORR_SIZE]; 2]),
            minor_correction_history: Box::new([[0; CORR_SIZE]; 2]),
            non_pawn_correction_history: Box::new([[[0; CORR_SIZE]; 2]; 2]),
            continuation_correction_history: vec![0; PIECE_TO_SIZE],
            countermove: Box::new([[Move::NULL; 64]; 64]),
            root_move_offset: 0,
            tt_write_mode: TtWriteMode::Main,
            multi_pv: 1,
            syzygy_probe_depth: 1,
            syzygy_probe_limit: 7,
            syzygy_50_move_rule: true,
            syzygy_largest: 0,
            root_iteration_nodes: 0,
            root_best_nodes: 0,
            root_best_effort: 0.0,
        }
    }
}

impl MovePicker {
    fn full(scored: ScoredMoveList) -> Self {
        Self::Full { scored, index: 0 }
    }

    fn staged(searcher: &Searcher, board: &mut Board, tt_move: Move, ply: usize) -> Self {
        let captures = board.generate_legal_captures();
        let (captures, bad_captures) =
            searcher.score_staged_captures(board, captures.as_slice(), tt_move);
        Self::Staged {
            captures,
            bad_captures,
            quiets: None,
            capture_index: 0,
            bad_capture_index: 0,
            quiet_index: 0,
            tt_move,
            ply,
        }
    }

    fn next(&mut self, searcher: &Searcher, board: &mut Board) -> Option<ScoredMove> {
        match self {
            Self::Full { scored, index } => {
                if *index >= scored.len() {
                    return None;
                }
                let picked = pick_next(scored.as_mut_slice(), *index);
                *index += 1;
                Some(picked)
            }
            Self::Staged {
                captures,
                bad_captures,
                quiets,
                capture_index,
                bad_capture_index,
                quiet_index,
                tt_move,
                ply,
            } => {
                if *capture_index < captures.len() {
                    let picked = pick_next(captures.as_mut_slice(), *capture_index);
                    *capture_index += 1;
                    return Some(picked);
                }
                if quiets.is_none() {
                    let quiet_moves = board.generate_legal_quiets();
                    *quiets =
                        Some(searcher.score_moves(board, quiet_moves.as_slice(), *tt_move, *ply));
                }
                let scored = quiets.as_mut().expect("quiets generated");
                if *quiet_index < scored.len() {
                    let picked = pick_next(scored.as_mut_slice(), *quiet_index);
                    *quiet_index += 1;
                    return Some(picked);
                }
                if *bad_capture_index < bad_captures.len() {
                    let picked = pick_next(bad_captures.as_mut_slice(), *bad_capture_index);
                    *bad_capture_index += 1;
                    return Some(picked);
                }
                None
            }
        }
    }
}

impl Searcher {
    pub(crate) fn worker_default() -> Self {
        Self {
            worker_pool: WorkerPool::default(),
            ..Self::default()
        }
    }

    pub(crate) fn reset_worker_state_for_new_game(&mut self) {
        self.clear_history();
        self.evaluator.clear_pawn_table();
    }

    pub(crate) fn run_worker_job<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        job: WorkerJob,
        poll: &mut P,
    ) -> SearchResult {
        self.tt = job.tt;
        self.hash_mb = job.hash_mb;
        self.root_move_offset = job.root_move_offset;
        self.tt_write_mode = TtWriteMode::Helper;
        self.search_worker(
            job.root,
            job.limits,
            job.engine_options,
            job.root_moves.as_ref(),
            poll,
        )
    }

    pub fn configure(&mut self, options: &SearchOptions) {
        self.configure_engine(&options.engine);
    }

    fn configure_engine(&mut self, options: &EngineOptions) {
        if options.hash_mb != self.hash_mb {
            if self.tt.resize(options.hash_mb) {
                self.hash_mb = options.hash_mb;
            } else {
                println!(
                    "info string Unable to allocate Hash value {}; keeping {} MiB.",
                    options.hash_mb, self.hash_mb
                );
            }
        }
        if options.clear_hash {
            self.tt.clear();
        }
        let old_path = syzygy::current_path();
        let largest = syzygy::initialize(&options.syzygy.path);
        if old_path != options.syzygy.path && !options.syzygy.path.is_empty() {
            if largest == 0 {
                println!("info string SyzygyPath loaded no usable tablebases.");
            } else {
                let (wdl, dtz) = syzygy::tablebase_file_counts(&options.syzygy.path);
                println!(
                    "info string Found {wdl} WDL and {dtz} DTZ tablebase files (up to {largest}-man)."
                );
            }
        }
        self.worker_pool
            .set_helper_count(options.threads.saturating_sub(1));
    }

    pub fn new_game(&mut self) {
        self.tt.clear();
        self.clear_history();
        self.evaluator.clear_pawn_table();
        self.worker_pool.new_game();
    }

    pub fn clear_history(&mut self) {
        self.main_history = Box::new([[[0; 64]; 64]; 2]);
        self.cap_history = Box::new([[[0; 6]; 64]; 6]);
        self.low_ply_history = Box::new([[[0; 64]; 64]; LOW_PLY_HISTORY_SIZE]);
        self.pawn_history.fill(0);
        self.cont_history_1.fill(0);
        self.cont_history_2.fill(0);
        self.cont_history_4.fill(0);
        self.cont_history_6.fill(0);
        self.correction_history = Box::new([[0; CORR_SIZE]; 2]);
        self.minor_correction_history = Box::new([[0; CORR_SIZE]; 2]);
        self.non_pawn_correction_history = Box::new([[[0; CORR_SIZE]; 2]; 2]);
        self.continuation_correction_history.fill(0);
        self.countermove = Box::new([[Move::NULL; 64]; 64]);
        self.killers = [[Move::NULL; 2]; MAX_PLY];
    }

    pub fn hashfull(&self) -> usize {
        self.tt.hashfull()
    }

    pub fn search(
        &mut self,
        root: Board,
        options: &SearchOptions,
        emit_info: bool,
        mut poll: impl FnMut() -> SearchEvent,
    ) -> SearchResult {
        self.search_impl::<true, _>(
            root,
            options.limits.clone(),
            options.engine.clone(),
            emit_info,
            &mut poll,
        )
    }

    fn search_impl<const ALLOW_PARALLEL: bool, P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        root: Board,
        limits: SearchLimits,
        engine_options: EngineOptions,
        emit_info: bool,
        poll: &mut P,
    ) -> SearchResult {
        if ALLOW_PARALLEL && engine_options.threads <= 1 && !self.tt.ensure_local(self.hash_mb) {
            println!(
                "info string Unable to restore local transposition table at {} MiB.",
                self.hash_mb
            );
        }
        self.root_move_offset = 0;
        self.tt_write_mode = TtWriteMode::Main;
        self.reset_search_state(&limits, &engine_options, root.side_to_move(), true, true);

        let board = root;
        let legal_moves = board.generate_legal_movelist();
        if legal_moves.is_empty() {
            return self.no_legal_moves_result(&board);
        }

        let filtered_root_moves;
        let root_candidates = if limits.search_moves.is_empty() {
            legal_moves.as_slice()
        } else {
            filtered_root_moves = legal_moves
                .iter()
                .copied()
                .filter(|mv| {
                    limits
                        .search_moves
                        .iter()
                        .any(|requested| mv.same_uci_move(*requested))
                })
                .collect::<Vec<_>>();
            if filtered_root_moves.is_empty() {
                legal_moves.as_slice()
            } else {
                filtered_root_moves.as_slice()
            }
        };

        let syzygy_root_moves = self.syzygy_root_moves(&board, root_candidates);
        let root_moves = syzygy_root_moves.as_deref().unwrap_or(root_candidates);

        if ALLOW_PARALLEL {
            let threads = engine_options
                .threads
                .clamp(1, MAX_THREADS)
                .min(root_moves.len().max(1));
            if threads > 1
                && engine_options.multi_pv <= 1
                && limits.nodes == 0
                && self.limits.depth.min(MAX_DEPTH - 1) >= MIN_PARALLEL_DEPTH
            {
                return self.search_parallel(
                    board,
                    root_moves,
                    limits,
                    engine_options.clone(),
                    threads,
                    emit_info,
                    poll,
                );
            }
        }

        self.search_root(board, root_moves, emit_info, poll)
    }

    fn reset_search_state(
        &mut self,
        limits: &SearchLimits,
        engine_options: &EngineOptions,
        side_to_move: Color,
        age_tt: bool,
        age_history: bool,
    ) {
        self.start = Instant::now();
        self.nodes = 0;
        self.tb_hits = 0;
        self.seldepth = 0;
        self.stopped = false;
        self.quit = false;
        self.pondering = limits.ponder;
        self.ponderhit = false;
        self.limits = compute_runtime_limits(limits, engine_options, side_to_move, MAX_DEPTH);
        self.multi_pv = engine_options.multi_pv.clamp(1, 256);
        self.syzygy_probe_depth = engine_options.syzygy.probe_depth;
        self.syzygy_probe_limit = engine_options.syzygy.probe_limit;
        self.syzygy_50_move_rule = engine_options.syzygy.fifty_move_rule;
        self.syzygy_largest = syzygy::largest().min(self.syzygy_probe_limit);
        self.root_iteration_nodes = 0;
        self.root_best_nodes = 0;
        self.root_best_effort = 0.0;
        if age_tt {
            self.tt.new_search();
        }
        if age_history {
            self.age_history();
        }
        self.pv_table = [[Move::NULL; MAX_PLY]; MAX_PLY];
        self.pv_len = [0; MAX_PLY];
        self.stack_moves = [Move::NULL; MAX_PLY];
        self.stack_pieces = [Piece::Pawn; MAX_PLY];
        self.stack_static_eval = [VALUE_NONE; MAX_PLY];
    }

    fn no_legal_moves_result(&mut self, board: &Board) -> SearchResult {
        let result = self.result_for_no_legal_moves(board);
        SearchResult {
            bestmove: Move::NULL,
            pondermove: Move::NULL,
            score: self
                .evaluator
                .evaluate_result(result, board.side_to_move(), 0),
            depth: 0,
            nodes: 0,
            tb_hits: self.tb_hits,
            elapsed_ms: self.start.elapsed().as_millis(),
            exit: SearchExit::Stop,
            ponderhit: self.ponderhit,
        }
    }

    fn syzygy_root_moves(&mut self, board: &Board, legal_moves: &[Move]) -> Option<Vec<Move>> {
        if !self.can_probe_syzygy_root(board) || board.can_declare_draw() || self.limits.nodes > 0 {
            return None;
        }

        let probe = syzygy::probe_root_moves(
            board,
            self.syzygy_50_move_rule,
            board.has_repeated_position(),
        )?;
        self.tb_hits += 1;

        let mut tb_moves = Vec::new();
        for probe_move in &probe.moves {
            let Some(mv) = syzygy::legal_move_from_root_probe(board, probe_move.root_move) else {
                continue;
            };
            if legal_moves.contains(&mv) {
                tb_moves.push((mv, probe_move.rank, probe_move.score));
            }
        }

        let best_rank = tb_moves.iter().map(|(_, rank, _)| *rank).max()?;
        let preferred_move = if self.multi_pv <= 1 && probe.used_dtz && best_rank != 0 {
            syzygy::probe_root(board, self.syzygy_50_move_rule)
                .and_then(|probe| probe.best_move)
                .and_then(|root_move| syzygy::legal_move_from_root_probe(board, root_move))
        } else {
            None
        };

        if best_rank != 0
            && let Some(preferred_move) = preferred_move
            && tb_moves
                .iter()
                .any(|(tb_move, rank, _)| *tb_move == preferred_move && *rank == best_rank)
        {
            self.tb_hits += 1;
            return Some(vec![preferred_move]);
        }

        let mut root_moves = Vec::with_capacity(legal_moves.len());
        for &legal_move in legal_moves {
            if tb_moves
                .iter()
                .any(|(tb_move, rank, _)| *tb_move == legal_move && *rank == best_rank)
            {
                root_moves.push(legal_move);
            }
        }

        if root_moves.is_empty() {
            None
        } else {
            Some(root_moves)
        }
    }

    fn search_root<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        mut board: Board,
        legal_moves: &[Move],
        emit_info: bool,
        poll: &mut P,
    ) -> SearchResult {
        self.root_moves.clear();
        self.root_moves.extend_from_slice(legal_moves);
        if self.multi_pv > 1 {
            return self.search_root_multipv(board, legal_moves, emit_info, poll);
        }
        let mut bestmove = legal_moves[0];
        let mut pondermove = Move::NULL;
        let mut best_score = -INF_SCORE;
        let mut completed_depth = 0;
        let max_depth = self.limits.depth.min(MAX_DEPTH - 1);
        let mut stable_best_depths = 0usize;
        let mut last_score_drop = 0;

        for depth in 1..=max_depth {
            let previous_bestmove = bestmove;
            self.root_iteration_nodes = self.nodes;
            self.root_best_nodes = 0;
            self.root_best_effort = 0.0;
            let use_aspiration = depth >= 4 && best_score.abs() < MATE_SCORE - MAX_PLY as i32;
            let mut alpha = if use_aspiration {
                best_score - 25
            } else {
                -INF_SCORE
            };
            let mut beta = if use_aspiration {
                best_score + 25
            } else {
                INF_SCORE
            };

            loop {
                let score = self.negamax(
                    &mut board,
                    depth as i32,
                    alpha,
                    beta,
                    0,
                    true,
                    true,
                    Move::NULL,
                    false,
                    poll,
                );
                if self.stopped || self.quit {
                    break;
                }
                if score <= alpha {
                    alpha = (alpha - 75).max(-INF_SCORE);
                    beta = (alpha + beta) / 2;
                    continue;
                }
                if score >= beta {
                    beta = (beta + 75).min(INF_SCORE);
                    continue;
                }
                last_score_drop = if completed_depth > 0 {
                    best_score - score
                } else {
                    0
                };
                best_score = score;
                completed_depth = depth;
                let iteration_nodes = self.nodes.saturating_sub(self.root_iteration_nodes).max(1);
                self.root_best_effort = self.root_best_nodes as f64 / iteration_nodes as f64;
                if self.pv_len[0] > 0 {
                    bestmove = self.pv_table[0][0];
                    pondermove = if self.pv_len[0] > 1 {
                        self.pv_table[0][1]
                    } else {
                        Move::NULL
                    };
                    if bestmove == previous_bestmove {
                        stable_best_depths += 1;
                    } else {
                        stable_best_depths = 0;
                    }
                }
                break;
            }

            if self.stopped || self.quit {
                break;
            }

            if emit_info {
                self.send_info(depth, best_score);
            }

            if legal_moves.len() == 1 && depth >= 2 {
                break;
            }

            if !self.pondering {
                let elapsed_ms = self.elapsed_ms();
                let effort_scale = if self.root_best_effort > 0.65 && stable_best_depths > 0 {
                    0.85
                } else if self.root_best_effort < 0.25 || stable_best_depths == 0 {
                    1.20
                } else {
                    1.0
                };
                let score_scale = if last_score_drop > 80 {
                    1.25
                } else if last_score_drop > 40 {
                    1.10
                } else {
                    1.0
                };
                let dynamic_soft_ms = self.limits.soft_ms * effort_scale * score_scale;
                if elapsed_ms >= self.limits.hard_ms
                    || (elapsed_ms >= dynamic_soft_ms
                        && (dynamic_soft_ms >= self.limits.hard_ms
                            || (stable_best_depths > 0 && last_score_drop <= 50)))
                {
                    break;
                }
            }
        }

        SearchResult {
            bestmove,
            pondermove,
            score: best_score,
            depth: completed_depth,
            nodes: self.nodes,
            tb_hits: self.tb_hits,
            elapsed_ms: self.start.elapsed().as_millis(),
            exit: if self.quit {
                SearchExit::Quit
            } else {
                SearchExit::Stop
            },
            ponderhit: self.ponderhit,
        }
    }

    fn search_root_multipv<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        mut board: Board,
        legal_moves: &[Move],
        emit_info: bool,
        poll: &mut P,
    ) -> SearchResult {
        let mut bestmove = legal_moves[0];
        let mut pondermove = Move::NULL;
        let mut best_score = -INF_SCORE;
        let mut completed_depth = 0;
        let max_depth = self.limits.depth.min(MAX_DEPTH - 1);

        for depth in 1..=max_depth {
            let mut lines = Vec::with_capacity(legal_moves.len());
            for &mv in legal_moves {
                if self.check_stop(poll) {
                    break;
                }

                let moving_piece = board.moving_piece(mv);
                self.stack_moves[0] = mv;
                self.stack_pieces[0] = moving_piece;
                board.make_move_unchecked(mv);
                self.tt.prefetch(board.hash);
                let score = -self.negamax(
                    &mut board,
                    depth as i32 - 1,
                    -INF_SCORE,
                    INF_SCORE,
                    1,
                    true,
                    true,
                    Move::NULL,
                    false,
                    poll,
                );
                board.unmake_move(mv);
                self.stack_moves[0] = Move::NULL;
                self.stack_pieces[0] = Piece::Pawn;

                if self.stopped || self.quit {
                    break;
                }

                let mut pv = Vec::with_capacity(MAX_PLY);
                pv.push(mv);
                let child_len = self.pv_len[1].min(MAX_PLY);
                for next_ply in 1..child_len {
                    let child = self.pv_table[1][next_ply];
                    if child.is_null() {
                        break;
                    }
                    pv.push(child);
                }
                lines.push(RootLine { score, pv });
            }

            if self.stopped || self.quit || lines.is_empty() {
                break;
            }

            lines.sort_by(|a, b| b.score.cmp(&a.score));
            let best = &lines[0];
            best_score = best.score;
            bestmove = best.pv[0];
            pondermove = best.pv.get(1).copied().unwrap_or(Move::NULL);
            completed_depth = depth;
            self.set_root_pv(&best.pv);

            if emit_info {
                let pv_count = self.multi_pv.min(lines.len());
                for (index, line) in lines.iter().take(pv_count).enumerate() {
                    self.send_info_line(depth, line.score, Some(index + 1), &line.pv);
                }
            }

            if !self.pondering {
                let elapsed_ms = self.elapsed_ms();
                if elapsed_ms >= self.limits.hard_ms || elapsed_ms >= self.limits.soft_ms {
                    break;
                }
            }
        }

        SearchResult {
            bestmove,
            pondermove,
            score: best_score,
            depth: completed_depth,
            nodes: self.nodes,
            tb_hits: self.tb_hits,
            elapsed_ms: self.start.elapsed().as_millis(),
            exit: if self.quit {
                SearchExit::Quit
            } else {
                SearchExit::Stop
            },
            ponderhit: self.ponderhit,
        }
    }

    fn search_worker<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        root: Board,
        limits: SearchLimits,
        engine_options: EngineOptions,
        legal_moves: &[Move],
        poll: &mut P,
    ) -> SearchResult {
        self.reset_search_state(&limits, &engine_options, root.side_to_move(), false, true);
        self.search_root(root, legal_moves, false, poll)
    }

    #[cold]
    #[inline(never)]
    fn search_parallel<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        root: Board,
        root_moves: &[Move],
        limits: SearchLimits,
        engine_options: EngineOptions,
        threads: usize,
        emit_info: bool,
        poll: &mut P,
    ) -> SearchResult {
        self.tt.make_shared();
        let stop_state = Arc::new(AtomicU8::new(STOP_NONE));
        let helper_count = threads.min(root_moves.len()).saturating_sub(1);
        let root_len = root_moves.len();
        let mut worker_engine_options = engine_options;
        worker_engine_options.threads = 1;
        self.worker_pool.set_helper_count(helper_count);
        let root_moves_shared: Arc<[Move]> = root_moves.to_vec().into();

        let (result_tx, result_rx) = mpsc::channel();
        let mut launched_helpers = 0usize;
        for index in 0..helper_count {
            let offset = ((index + 1) * root_len / threads).max(1) % root_len;
            let job = WorkerJob {
                root: root.clone(),
                root_moves: Arc::clone(&root_moves_shared),
                limits: limits.clone(),
                engine_options: worker_engine_options.clone(),
                tt: self.tt.clone(),
                hash_mb: self.hash_mb,
                root_move_offset: offset,
                stop_state: Arc::clone(&stop_state),
                result_tx: result_tx.clone(),
            };
            if self.worker_pool.send_search(index, job) {
                launched_helpers += 1;
            }
        }
        drop(result_tx);

        self.root_move_offset = 0;
        let mut main_poll = || match stop_state.load(Ordering::Relaxed) {
            STOP_QUIT => SearchEvent::Quit,
            STOP_SEARCH => SearchEvent::Stop,
            _ => match poll() {
                SearchEvent::Quit => {
                    stop_state.store(STOP_QUIT, Ordering::Relaxed);
                    SearchEvent::Quit
                }
                SearchEvent::Stop => {
                    stop_state.store(STOP_SEARCH, Ordering::Relaxed);
                    SearchEvent::Stop
                }
                SearchEvent::PonderHit => SearchEvent::PonderHit,
                SearchEvent::None => SearchEvent::None,
            },
        };
        let main_result = self.search_root(root, root_moves, emit_info, &mut main_poll);
        stop_state
            .compare_exchange(STOP_NONE, STOP_SEARCH, Ordering::Relaxed, Ordering::Relaxed)
            .ok();

        let mut helper_results = Vec::with_capacity(launched_helpers + 1);
        helper_results.push(main_result);
        for _ in 0..launched_helpers {
            if let Ok(result) = result_rx.recv() {
                helper_results.push(result);
            }
        }
        self.root_move_offset = 0;

        let mut total_nodes = 0u64;
        let mut total_tb_hits = 0u64;
        let mut quit = false;
        for result in &helper_results {
            total_nodes = total_nodes.saturating_add(result.nodes);
            total_tb_hits = total_tb_hits.saturating_add(result.tb_hits);
            quit |= result.exit == SearchExit::Quit;
        }
        let mut best =
            select_parallel_result(&helper_results, root_moves).unwrap_or(SearchResult {
                bestmove: root_moves[0],
                pondermove: Move::NULL,
                score: -INF_SCORE,
                depth: 0,
                nodes: 0,
                tb_hits: 0,
                elapsed_ms: self.start.elapsed().as_millis(),
                exit: SearchExit::Stop,
                ponderhit: self.ponderhit,
            });
        self.nodes = total_nodes;
        self.tb_hits = total_tb_hits;
        self.quit = quit;
        self.stopped = true;
        best.nodes = total_nodes;
        best.tb_hits = total_tb_hits;
        best.elapsed_ms = self.start.elapsed().as_millis();
        best.ponderhit = self.ponderhit || helper_results.iter().any(|result| result.ponderhit);
        best.exit = if quit {
            SearchExit::Quit
        } else {
            SearchExit::Stop
        };
        best
    }

    fn negamax<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        board: &mut Board,
        mut depth: i32,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        is_pv: bool,
        allow_null: bool,
        excluded: Move,
        cut_node: bool,
        poll: &mut P,
    ) -> i32 {
        if self.check_stop(poll) {
            return 0;
        }
        if ply >= MAX_PLY - 1 {
            return self.corrected_eval(board, ply);
        }
        self.pv_len[ply] = ply;
        self.seldepth = self.seldepth.max(ply);

        if ply > 0 && board.can_declare_draw_in_search() {
            return 0;
        }

        let in_check = board.is_in_check();
        if in_check {
            depth += 1;
        }

        let mate_alpha = -MATE_SCORE + ply as i32;
        let mate_beta = MATE_SCORE - ply as i32 - 1;
        alpha = alpha.max(mate_alpha);
        let beta = beta.min(mate_beta);
        if alpha >= beta {
            return alpha;
        }

        if depth <= 0 {
            return self.quiescence(board, alpha, beta, ply, 0, poll);
        }

        let original_alpha = alpha;
        let hash = board.hash;
        if let Some(score) = self.syzygy_wdl_score(board, depth, ply, excluded) {
            self.store_tt(
                hash,
                depth,
                score,
                Bound::Exact,
                Move::NULL,
                ply,
                VALUE_NONE,
            );
            return score;
        }
        let tt_entry = self.tt.probe(hash);
        let tt_move = tt_entry
            .and_then(|entry| entry.best_move())
            .unwrap_or(Move::NULL);
        let tt_score = tt_entry
            .map(|entry| score_from_tt(entry.score as i32, ply, board.halfmove_clock))
            .unwrap_or(VALUE_NONE);
        let tt_depth = tt_entry.map(|entry| entry.depth as i32).unwrap_or(-1);
        let tt_bound = tt_entry.and_then(|entry| entry.bound());
        if !is_pv
            && excluded.is_null()
            && let Some(entry) = tt_entry
            && entry.depth as i32 >= depth
            && let Some(bound) = entry.bound()
        {
            let score = score_from_tt(entry.score as i32, ply, board.halfmove_clock);
            match bound {
                Bound::Exact => return score,
                Bound::Lower if score >= beta => return score,
                Bound::Upper if score <= alpha => return score,
                _ => {}
            }
        }

        if !is_pv && excluded.is_null() && depth >= 4 && (tt_move.is_null() || tt_depth < depth - 3)
        {
            depth -= 1;
        }

        let (static_eval, raw_static_eval) = if in_check {
            (VALUE_NONE, VALUE_NONE)
        } else if let Some(entry) = tt_entry {
            if entry.static_eval as i32 != VALUE_NONE {
                let raw = entry.static_eval as i32;
                (self.corrected_eval_from_raw(board, raw, ply), raw)
            } else {
                let raw = self.raw_eval(board);
                (self.corrected_eval_from_raw(board, raw, ply), raw)
            }
        } else {
            let raw = self.raw_eval(board);
            (self.corrected_eval_from_raw(board, raw, ply), raw)
        };
        self.stack_static_eval[ply] = static_eval;
        let improving = !in_check
            && ply >= 2
            && self.stack_static_eval[ply - 2] != VALUE_NONE
            && static_eval > self.stack_static_eval[ply - 2];
        let improving_i = if improving { 1 } else { 0 };
        let not_improving_i = 1 - improving_i;
        let eval_for_pruning = if !in_check && tt_score != VALUE_NONE {
            match tt_bound {
                Some(Bound::Exact) => tt_score,
                Some(Bound::Lower) if tt_score > static_eval => tt_score,
                Some(Bound::Upper) if tt_score < static_eval => tt_score,
                _ => static_eval,
            }
        } else {
            static_eval
        };
        if !is_pv && !in_check && excluded.is_null() {
            let futility_margin = (70 + 20 * not_improving_i) * depth;
            if depth <= 8 && eval_for_pruning - futility_margin >= beta {
                return eval_for_pruning;
            }
            if depth <= 3 && eval_for_pruning + 150 * depth < alpha {
                return self.quiescence(board, alpha, beta, ply, 0, poll);
            }
            if allow_null
                && depth >= 3
                && eval_for_pruning >= beta - 12 * depth - 24 * improving_i
                && board.has_non_pawn_material(board.side_to_move())
            {
                let reduction = 4 + depth / 4 + ((eval_for_pruning - beta) / 200).clamp(0, 3);
                board.make_null_move();
                self.tt.prefetch(board.hash);
                let score = -self.negamax(
                    board,
                    depth - reduction,
                    -beta,
                    -beta + 1,
                    ply + 1,
                    false,
                    false,
                    Move::NULL,
                    true,
                    poll,
                );
                board.unmake_null_move();
                if self.stopped || self.quit {
                    return 0;
                }
                if score >= beta {
                    if depth >= 10 {
                        let verify_depth = (depth - reduction).max(1);
                        let verified = self.negamax(
                            board,
                            verify_depth,
                            beta - 1,
                            beta,
                            ply,
                            false,
                            false,
                            Move::NULL,
                            false,
                            poll,
                        );
                        if self.stopped || self.quit {
                            return 0;
                        }
                        if verified < beta {
                            // Continue normally when the null cutoff is not stable
                            // under a verification search with null move disabled.
                        } else {
                            return score;
                        }
                    } else {
                        return score;
                    }
                }
            }

            if depth >= 4 {
                let probcut_beta = beta + 160;
                let captures = board.generate_legal_captures();
                let mut scored = self.score_tactical_moves(board, captures.as_slice(), tt_move);
                for index in 0..scored.len().min(8) {
                    let picked = pick_next(scored.as_mut_slice(), index);
                    let mv = picked.mv;
                    if !board.see_ge(mv, 0) {
                        continue;
                    }
                    self.stack_moves[ply] = mv;
                    board.make_move_unchecked(mv);
                    self.tt.prefetch(board.hash);
                    let score =
                        -self.quiescence(board, -probcut_beta, -probcut_beta + 1, ply + 1, 0, poll);
                    let score = if score >= probcut_beta {
                        -self.negamax(
                            board,
                            depth - 4,
                            -probcut_beta,
                            -probcut_beta + 1,
                            ply + 1,
                            false,
                            false,
                            Move::NULL,
                            true,
                            poll,
                        )
                    } else {
                        score
                    };
                    board.unmake_move(mv);
                    self.stack_moves[ply] = Move::NULL;
                    if self.stopped || self.quit {
                        return 0;
                    }
                    if score >= probcut_beta {
                        let cutoff_score = score - (probcut_beta - beta);
                        self.store_tt(
                            hash,
                            depth - 3,
                            cutoff_score,
                            Bound::Lower,
                            mv,
                            ply,
                            raw_static_eval,
                        );
                        return cutoff_score;
                    }
                }
            }
        }

        let mut move_picker = if in_check || ply == 0 || !excluded.is_null() {
            let legal_moves = board.generate_legal_movelist();
            if legal_moves.is_empty() {
                return if in_check {
                    -MATE_SCORE + ply as i32
                } else {
                    0
                };
            }

            let root_moves;
            let legal_moves = if ply == 0 && !self.root_moves.is_empty() {
                root_moves = legal_moves
                    .iter()
                    .copied()
                    .filter(|mv| self.root_moves.contains(mv))
                    .collect::<Vec<_>>();
                if root_moves.is_empty() {
                    legal_moves.as_slice()
                } else {
                    root_moves.as_slice()
                }
            } else {
                legal_moves.as_slice()
            };

            let mut scored = self.score_moves(board, legal_moves, tt_move, ply);
            if ply == 0 && self.root_move_offset > 0 && scored.len() > 1 {
                let offset = self.root_move_offset % scored.len();
                diversify_root_scores(scored.as_mut_slice(), offset);
            }
            MovePicker::full(scored)
        } else {
            MovePicker::staged(self, board, tt_move, ply)
        };
        let mut best_move = Move::NULL;
        let mut best_score = -INF_SCORE;
        let mut searched = 0usize;
        let mut legal_move_seen = false;
        let mut quiets = crate::board::MoveList::new();
        let mut bad_caps = BadCaptureList::new();
        let previous_move = if ply > 0 {
            self.stack_moves[ply - 1]
        } else {
            Move::NULL
        };

        while let Some(picked) = move_picker.next(self, board) {
            let mv = picked.mv;
            if mv == excluded {
                continue;
            }
            legal_move_seen = true;
            let is_capture = mv.is_capture();
            let is_quiet = board.is_quiet_move(mv);
            let see = if is_capture { picked.see as i32 } else { 0 };
            let moving_piece = board.moving_piece(mv);
            let captured_piece = board.captured_piece(mv);
            let quiet_hist = if is_quiet {
                self.quiet_history_score(board, board.side_to_move(), mv, ply)
            } else {
                0
            };
            let mut gives_check = None;

            if !is_pv && !in_check && searched > 0 {
                if is_quiet {
                    let prune_margin = (90 + 25 * not_improving_i) * depth;
                    let prune_candidate = (depth <= 3 && eval_for_pruning + prune_margin <= alpha)
                        || (depth <= 8 && searched > late_move_prune_count(depth, improving))
                        || (depth <= 4 && quiet_hist < -10_000)
                        || (depth <= 6 && quiet_hist < -4_000 * depth);
                    if prune_candidate && !move_gives_check(board, mv, &mut gives_check) {
                        continue;
                    }
                } else if depth <= 7
                    && see < 0
                    && !board.see_ge(mv, -80 * depth)
                    && !move_gives_check(board, mv, &mut gives_check)
                {
                    continue;
                }
            }

            let child_is_pv = is_pv && searched == 0;
            let mut extension = 0;
            if ply > 0
                && mv == tt_move
                && excluded.is_null()
                && depth >= 5
                && tt_depth >= depth - 3
                && matches!(tt_bound, Some(Bound::Lower | Bound::Exact))
                && tt_score.abs() < MATE_SCORE - MAX_PLY as i32
            {
                let singular_beta = tt_score - 2 * depth;
                let singular_depth = (depth - 1) / 2;
                let singular_score = self.negamax(
                    board,
                    singular_depth,
                    singular_beta - 1,
                    singular_beta,
                    ply,
                    false,
                    false,
                    mv,
                    false,
                    poll,
                );
                if self.stopped || self.quit {
                    return 0;
                }
                if singular_score < singular_beta {
                    extension = if !is_pv && singular_score < singular_beta - 20 {
                        2
                    } else {
                        1
                    };
                } else if singular_beta >= beta {
                    return singular_beta;
                } else if tt_score >= beta {
                    extension = -1;
                }
            }

            let checking_move =
                if depth >= 3 && searched >= 2 && (is_quiet || see < 0) && !mv.is_promo() {
                    move_gives_check(board, mv, &mut gives_check)
                } else {
                    gives_check.unwrap_or(false)
                };

            self.stack_moves[ply] = mv;
            self.stack_pieces[ply] = moving_piece;
            let nodes_before_move = if ply == 0 { self.nodes } else { 0 };
            board.make_move_unchecked(mv);
            self.tt.prefetch(board.hash);
            let new_depth = depth - 1 + extension;
            let mut score;

            if searched == 0 {
                score = -self.negamax(
                    board,
                    new_depth,
                    -beta,
                    -alpha,
                    ply + 1,
                    child_is_pv,
                    true,
                    Move::NULL,
                    !child_is_pv && !cut_node,
                    poll,
                );
            } else {
                let reducible = depth >= 3
                    && searched >= 2
                    && !in_check
                    && (is_quiet || see < 0)
                    && !mv.is_promo()
                    && !checking_move;
                if reducible {
                    let hist = quiet_hist;
                    let mut reduction = lmr_reduction(depth, searched);
                    if is_pv {
                        reduction -= 1;
                    } else if is_quiet {
                        reduction += 1;
                    }
                    if improving {
                        reduction -= 1;
                    }
                    if !tt_move.is_null() && searched >= 4 {
                        reduction += 1;
                    }
                    if cut_node {
                        reduction += 1;
                    }
                    if !is_quiet && see < 0 {
                        reduction += 1;
                    }
                    if !is_pv && !cut_node && quiet_hist > 4_000 {
                        reduction -= 1;
                    }
                    reduction -= hist / 8_192;
                    reduction = reduction.clamp(1, new_depth.max(1));
                    score = -self.negamax(
                        board,
                        new_depth - reduction,
                        -alpha - 1,
                        -alpha,
                        ply + 1,
                        false,
                        true,
                        Move::NULL,
                        true,
                        poll,
                    );
                    if score > alpha {
                        score = -self.negamax(
                            board,
                            new_depth,
                            -alpha - 1,
                            -alpha,
                            ply + 1,
                            false,
                            true,
                            Move::NULL,
                            true,
                            poll,
                        );
                    }
                } else {
                    score = -self.negamax(
                        board,
                        new_depth,
                        -alpha - 1,
                        -alpha,
                        ply + 1,
                        false,
                        true,
                        Move::NULL,
                        true,
                        poll,
                    );
                }
                if score > alpha && score < beta {
                    score = -self.negamax(
                        board,
                        new_depth,
                        -beta,
                        -alpha,
                        ply + 1,
                        true,
                        true,
                        Move::NULL,
                        false,
                        poll,
                    );
                }
            }
            board.unmake_move(mv);
            self.stack_moves[ply] = Move::NULL;

            if self.stopped || self.quit {
                return 0;
            }

            let move_nodes = if ply == 0 {
                self.nodes.saturating_sub(nodes_before_move)
            } else {
                0
            };
            searched += 1;
            if score > best_score {
                best_score = score;
                best_move = mv;
                if ply == 0 {
                    self.root_best_nodes = move_nodes;
                }
            }
            if score > alpha {
                alpha = score;
                self.pv_table[ply][ply] = mv;
                let child_len = self.pv_len[ply + 1].max(ply + 1);
                for next_ply in ply + 1..child_len {
                    self.pv_table[ply][next_ply] = self.pv_table[ply + 1][next_ply];
                }
                self.pv_len[ply] = child_len;

                if score >= beta {
                    if excluded.is_null() {
                        if !is_capture {
                            self.update_cutoff_tables(
                                board,
                                mv,
                                moving_piece,
                                previous_move,
                                ply,
                                depth,
                                quiets.as_slice(),
                                &bad_caps,
                            );
                        } else {
                            self.update_capture_history(
                                moving_piece,
                                mv.to_sq().index(),
                                captured_piece,
                                depth * depth,
                            );
                        }
                        self.store_tt(hash, depth, score, Bound::Lower, mv, ply, raw_static_eval);
                    }
                    return score;
                }
            }

            if is_quiet {
                quiets.push(mv);
            } else if is_capture && see < 0 {
                bad_caps.push(moving_piece, mv.to_sq().index(), captured_piece);
            }
        }

        if !legal_move_seen {
            return if in_check {
                -MATE_SCORE + ply as i32
            } else {
                0
            };
        }

        let bound = if best_score > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        if bound == Bound::Exact
            && excluded.is_null()
            && !in_check
            && static_eval != VALUE_NONE
            && best_score.abs() < MATE_SCORE - MAX_PLY as i32
        {
            self.update_correction(board, best_score - static_eval, depth, ply);
        }
        if excluded.is_null() {
            self.store_tt(hash, depth, alpha, bound, best_move, ply, raw_static_eval);
        }
        alpha
    }

    fn quiescence<P: FnMut() -> SearchEvent + ?Sized>(
        &mut self,
        board: &mut Board,
        mut alpha: i32,
        beta: i32,
        ply: usize,
        qply: usize,
        poll: &mut P,
    ) -> i32 {
        if self.check_stop(poll) {
            return 0;
        }
        self.pv_len[ply] = ply;
        self.seldepth = self.seldepth.max(ply);

        if board.can_declare_draw_in_search() {
            return 0;
        }

        let in_check = board.is_in_check();
        let hash = board.hash;
        let original_alpha = alpha;
        let tt_entry = self.tt.probe(hash);
        let tt_move = tt_entry
            .and_then(|entry| entry.best_move())
            .unwrap_or(Move::NULL);
        if let Some(entry) = tt_entry
            && entry.depth >= 0
            && let Some(bound) = entry.bound()
        {
            let score = score_from_tt(entry.score as i32, ply, board.halfmove_clock);
            match bound {
                Bound::Exact => return score,
                Bound::Lower if score >= beta => return score,
                Bound::Upper if score <= alpha => return score,
                _ => {}
            }
        }

        let mut q_raw_static_eval = VALUE_NONE;
        let mut stand_pat_for_pruning = VALUE_NONE;
        if !in_check {
            let (stand_pat, raw_stand_pat) = if let Some(entry) = tt_entry {
                if entry.static_eval as i32 != VALUE_NONE {
                    let raw = entry.static_eval as i32;
                    (self.corrected_eval_from_raw(board, raw, ply), raw)
                } else {
                    let raw = self.raw_eval(board);
                    (self.corrected_eval_from_raw(board, raw, ply), raw)
                }
            } else {
                let raw = self.raw_eval(board);
                (self.corrected_eval_from_raw(board, raw, ply), raw)
            };
            q_raw_static_eval = raw_stand_pat;
            stand_pat_for_pruning = stand_pat;
            if stand_pat >= beta {
                self.store_tt(
                    hash,
                    0,
                    stand_pat,
                    Bound::Lower,
                    Move::NULL,
                    ply,
                    q_raw_static_eval,
                );
                return stand_pat;
            }
            if qply >= MAX_QPLY {
                return stand_pat.max(alpha);
            }
            if stand_pat > alpha {
                alpha = stand_pat;
            }
            if board.occupied_count() > 8 && stand_pat + piece_value(Piece::Queen) + 200 < alpha {
                return alpha;
            }
        }

        let moves = if in_check {
            board.generate_legal_movelist()
        } else {
            board.generate_legal_captures()
        };

        if in_check && moves.is_empty() {
            return -MATE_SCORE + ply as i32;
        }

        let mut best_move = Move::NULL;
        let mut scored = if in_check {
            self.score_moves(board, moves.as_slice(), tt_move, ply)
        } else {
            self.score_tactical_moves(board, moves.as_slice(), tt_move)
        };
        let mut tactical_count = 0usize;
        for index in 0..scored.len() {
            let picked = pick_next(scored.as_mut_slice(), index);
            let mv = picked.mv;
            if !in_check {
                let mut gives_check = None;
                tactical_count += 1;
                if !mv.is_promo()
                    && stand_pat_for_pruning != VALUE_NONE
                    && stand_pat_for_pruning
                        + board.captured_piece(mv).map(piece_value).unwrap_or(0)
                        + 150
                        <= alpha
                    && !move_gives_check(board, mv, &mut gives_check)
                {
                    continue;
                }
                if !mv.is_promo()
                    && tactical_count > 6
                    && picked.see < 0
                    && !move_gives_check(board, mv, &mut gives_check)
                {
                    continue;
                }
                if !mv.is_promo() {
                    let see_threshold = (alpha - stand_pat_for_pruning - 200).clamp(-800, 200);
                    if !board.see_ge(mv, see_threshold) {
                        continue;
                    }
                }
                if picked.see < 0 && !board.see_ge(mv, -50) {
                    continue;
                }
            }
            let moving_piece = board.moving_piece(mv);
            self.stack_moves[ply] = mv;
            self.stack_pieces[ply] = moving_piece;
            board.make_move_unchecked(mv);
            self.tt.prefetch(board.hash);
            let score = -self.quiescence(board, -beta, -alpha, ply + 1, qply + 1, poll);
            board.unmake_move(mv);
            self.stack_moves[ply] = Move::NULL;
            if self.stopped || self.quit {
                return 0;
            }
            if score >= beta {
                self.store_tt(hash, 0, score, Bound::Lower, mv, ply, q_raw_static_eval);
                return score;
            }
            if score > alpha {
                alpha = score;
                best_move = mv;
                self.pv_table[ply][ply] = mv;
                let child_len = self.pv_len[ply + 1].max(ply + 1);
                for next_ply in ply + 1..child_len {
                    self.pv_table[ply][next_ply] = self.pv_table[ply + 1][next_ply];
                }
                self.pv_len[ply] = child_len;
            }
        }
        let bound = if alpha > original_alpha {
            Bound::Exact
        } else {
            Bound::Upper
        };
        self.store_tt(hash, 0, alpha, bound, best_move, ply, q_raw_static_eval);
        alpha
    }

    fn score_moves(
        &self,
        board: &Board,
        moves: &[Move],
        tt_move: Move,
        ply: usize,
    ) -> ScoredMoveList {
        let mut scored = ScoredMoveList::new();
        let previous = if ply > 0 {
            self.stack_moves[ply - 1]
        } else {
            Move::NULL
        };
        let counter = if !previous.is_null() {
            self.countermove[previous.from_sq().index()][previous.to_sq().index()]
        } else {
            Move::NULL
        };

        for &mv in moves {
            let mut see = 0;
            let score = if mv == tt_move {
                30_000_000
            } else if mv.is_capture() {
                let attacker = board.moving_piece(mv);
                let victim = board.captured_piece(mv).unwrap_or(Piece::Pawn);
                see = board.see(mv);
                let hist =
                    self.cap_history[attacker as usize][mv.to_sq().index()][victim as usize] as i32;
                if see >= 0 {
                    20_000_000 + 32 * see + 10 * piece_value(victim) - piece_value(attacker) + hist
                } else {
                    -2_000_000 + see + hist
                }
            } else if mv.is_promo() {
                18_000_000 + piece_value(mv.promo_piece())
            } else if mv == self.killers[ply][0] {
                16_000_000
            } else if mv == self.killers[ply][1] {
                15_900_000
            } else if mv == counter {
                15_800_000
            } else {
                self.quiet_history_score(board, board.side_to_move(), mv, ply)
            };
            scored.push(mv, score, see);
        }
        scored
    }

    fn score_staged_captures(
        &self,
        board: &Board,
        moves: &[Move],
        tt_move: Move,
    ) -> (ScoredMoveList, ScoredMoveList) {
        let mut good = ScoredMoveList::new();
        let mut bad = ScoredMoveList::new();
        for &mv in moves {
            let scored = self.score_tactical_move(board, mv, tt_move);
            if mv == tt_move || scored.see >= 0 || mv.is_promo() {
                good.push(scored.mv, scored.score, scored.see as i32);
            } else {
                bad.push(scored.mv, scored.score, scored.see as i32);
            }
        }
        (good, bad)
    }

    fn score_tactical_moves(&self, board: &Board, moves: &[Move], tt_move: Move) -> ScoredMoveList {
        let mut scored = ScoredMoveList::new();
        for &mv in moves {
            let scored_move = self.score_tactical_move(board, mv, tt_move);
            scored.push(scored_move.mv, scored_move.score, scored_move.see as i32);
        }
        scored
    }

    fn score_tactical_move(&self, board: &Board, mv: Move, tt_move: Move) -> ScoredMove {
        let mut see = 0;
        let score = if mv == tt_move {
            if mv.is_capture() && !board.see_ge(mv, 0) {
                see = -1;
            }
            30_000_000
        } else if mv.is_capture() {
            let attacker = board.moving_piece(mv);
            let victim = board.captured_piece(mv).unwrap_or(Piece::Pawn);
            let promo_gain = if mv.is_promo() {
                piece_value(mv.promo_piece()) - piece_value(Piece::Pawn)
            } else {
                0
            };
            let hist =
                self.cap_history[attacker as usize][mv.to_sq().index()][victim as usize] as i32;
            if board.see_ge(mv, 0) {
                20_000_000 + 16 * (piece_value(victim) + promo_gain) - piece_value(attacker) + hist
            } else {
                see = -1;
                -2_000_000 + 16 * (piece_value(victim) + promo_gain) - piece_value(attacker) + hist
            }
        } else if mv.is_promo() {
            18_000_000 + piece_value(mv.promo_piece())
        } else {
            0
        };

        ScoredMove {
            mv,
            score,
            see: see as i16,
        }
    }

    fn quiet_history_score(&self, board: &Board, color: Color, mv: Move, ply: usize) -> i32 {
        let from = mv.from_sq().index();
        let to = mv.to_sq().index();
        let main = self.main_history[color as usize][from][to] as i32;
        let piece = board.moving_piece(mv) as usize;
        let pawn = self.pawn_history[pawn_history_index(board.pawn_key(), piece, to)] as i32;
        let low_ply = if ply < LOW_PLY_HISTORY_SIZE {
            self.low_ply_history[ply][from][to] as i32 / (1 + ply as i32)
        } else {
            0
        };
        2 * main + pawn + low_ply + self.cont_score(ply, piece, to)
    }

    fn cont_score(&self, ply: usize, piece: usize, to: usize) -> i32 {
        let mut score = 0;
        if ply >= 1 {
            let prev = self.stack_moves[ply - 1];
            if !prev.is_null() {
                score += self.cont_history_1[cont_index(
                    self.stack_pieces[ply - 1] as usize,
                    prev.to_sq().index(),
                    piece,
                    to,
                )] as i32;
            }
        }
        if ply >= 2 {
            let prev = self.stack_moves[ply - 2];
            if !prev.is_null() {
                score += self.cont_history_2[cont_index(
                    self.stack_pieces[ply - 2] as usize,
                    prev.to_sq().index(),
                    piece,
                    to,
                )] as i32;
            }
        }
        if ply >= 4 {
            let prev = self.stack_moves[ply - 4];
            if !prev.is_null() {
                score += self.cont_history_4[cont_index(
                    self.stack_pieces[ply - 4] as usize,
                    prev.to_sq().index(),
                    piece,
                    to,
                )] as i32;
            }
        }
        if ply >= 6 {
            let prev = self.stack_moves[ply - 6];
            if !prev.is_null() {
                score += self.cont_history_6[cont_index(
                    self.stack_pieces[ply - 6] as usize,
                    prev.to_sq().index(),
                    piece,
                    to,
                )] as i32;
            }
        }
        score
    }

    fn update_cutoff_tables(
        &mut self,
        board: &Board,
        best: Move,
        best_piece: Piece,
        previous: Move,
        ply: usize,
        depth: i32,
        quiets: &[Move],
        bad_caps: &BadCaptureList,
    ) {
        if self.killers[ply][0] != best {
            self.killers[ply][1] = self.killers[ply][0];
            self.killers[ply][0] = best;
        }

        let color = board.side_to_move();
        let pawn_key = board.pawn_key();
        let bonus = history_bonus(depth);
        self.update_quiet_history(color, best, best_piece, pawn_key, ply, bonus);
        for &quiet in quiets {
            let quiet_piece = board.moving_piece(quiet);
            self.update_quiet_history(color, quiet, quiet_piece, pawn_key, ply, -bonus);
        }
        for bad_cap in bad_caps.as_slice() {
            self.update_capture_history(bad_cap.attacker, bad_cap.to, bad_cap.captured, -bonus);
        }

        if !previous.is_null() {
            self.countermove[previous.from_sq().index()][previous.to_sq().index()] = best;
        }

        let piece = best_piece as usize;
        let to = best.to_sq().index();
        if ply >= 1 {
            let prev = self.stack_moves[ply - 1];
            if !prev.is_null() {
                update_hist_entry(
                    &mut self.cont_history_1[cont_index(
                        self.stack_pieces[ply - 1] as usize,
                        prev.to_sq().index(),
                        piece,
                        to,
                    )],
                    bonus,
                    HISTORY_MAX,
                );
            }
        }
        if ply >= 2 {
            let prev = self.stack_moves[ply - 2];
            if !prev.is_null() {
                update_hist_entry(
                    &mut self.cont_history_2[cont_index(
                        self.stack_pieces[ply - 2] as usize,
                        prev.to_sq().index(),
                        piece,
                        to,
                    )],
                    bonus,
                    HISTORY_MAX,
                );
            }
        }
        if ply >= 4 {
            let prev = self.stack_moves[ply - 4];
            if !prev.is_null() {
                update_hist_entry(
                    &mut self.cont_history_4[cont_index(
                        self.stack_pieces[ply - 4] as usize,
                        prev.to_sq().index(),
                        piece,
                        to,
                    )],
                    bonus / 2,
                    HISTORY_MAX,
                );
            }
        }
        if ply >= 6 {
            let prev = self.stack_moves[ply - 6];
            if !prev.is_null() {
                update_hist_entry(
                    &mut self.cont_history_6[cont_index(
                        self.stack_pieces[ply - 6] as usize,
                        prev.to_sq().index(),
                        piece,
                        to,
                    )],
                    bonus / 3,
                    HISTORY_MAX,
                );
            }
        }
    }

    fn update_quiet_history(
        &mut self,
        color: Color,
        mv: Move,
        piece: Piece,
        pawn_key: u64,
        ply: usize,
        bonus: i32,
    ) {
        update_hist_entry(
            &mut self.main_history[color as usize][mv.from_sq().index()][mv.to_sq().index()],
            bonus,
            HISTORY_MAX,
        );
        if ply < LOW_PLY_HISTORY_SIZE {
            update_hist_entry(
                &mut self.low_ply_history[ply][mv.from_sq().index()][mv.to_sq().index()],
                bonus,
                HISTORY_MAX,
            );
        }
        update_hist_entry(
            &mut self.pawn_history
                [pawn_history_index(pawn_key, piece as usize, mv.to_sq().index())],
            bonus,
            HISTORY_MAX,
        );
    }

    fn update_capture_history(
        &mut self,
        attacker: Piece,
        to: usize,
        captured: Option<Piece>,
        bonus: i32,
    ) {
        if let Some(captured) = captured {
            update_hist_entry(
                &mut self.cap_history[attacker as usize][to][captured as usize],
                bonus,
                CAP_HISTORY_MAX,
            );
        }
    }

    fn age_history(&mut self) {
        for color in self.main_history.iter_mut() {
            for from in color.iter_mut() {
                for value in from.iter_mut() {
                    *value /= 2;
                }
            }
        }
        for attacker in self.cap_history.iter_mut() {
            for to in attacker.iter_mut() {
                for value in to.iter_mut() {
                    *value /= 2;
                }
            }
        }
        for ply in self.low_ply_history.iter_mut() {
            for from in ply.iter_mut() {
                for value in from.iter_mut() {
                    *value /= 2;
                }
            }
        }
        for value in &mut self.pawn_history {
            *value /= 2;
        }
        for value in &mut self.cont_history_1 {
            *value /= 2;
        }
        for value in &mut self.cont_history_2 {
            *value /= 2;
        }
        for value in &mut self.cont_history_4 {
            *value /= 2;
        }
        for value in &mut self.cont_history_6 {
            *value /= 2;
        }
        for color in self.correction_history.iter_mut() {
            for value in color.iter_mut() {
                *value /= 2;
            }
        }
        for color in self.minor_correction_history.iter_mut() {
            for value in color.iter_mut() {
                *value /= 2;
            }
        }
        for stm in self.non_pawn_correction_history.iter_mut() {
            for color in stm.iter_mut() {
                for value in color.iter_mut() {
                    *value /= 2;
                }
            }
        }
        for value in &mut self.continuation_correction_history {
            *value /= 2;
        }
    }

    fn corrected_eval(&mut self, board: &Board, ply: usize) -> i32 {
        let raw = self.raw_eval(board);
        self.corrected_eval_from_raw(board, raw, ply)
    }

    fn raw_eval(&mut self, board: &Board) -> i32 {
        self.evaluator.evaluate(board)
    }

    fn corrected_eval_from_raw(&self, board: &Board, raw: i32, ply: usize) -> i32 {
        raw + self.correction_value(board, ply)
    }

    fn correction_value(&self, board: &Board, ply: usize) -> i32 {
        let color = board.side_to_move();
        let us = color as usize;
        let them = (!color) as usize;
        let pawn = self.correction_history[us][board.pawn_key() as usize & (CORR_SIZE - 1)] as i32;
        let minor =
            self.minor_correction_history[us][board.minor_key() as usize & (CORR_SIZE - 1)] as i32;
        let own_non_pawn = self.non_pawn_correction_history[us][us]
            [board.non_pawn_key(color) as usize & (CORR_SIZE - 1)]
            as i32;
        let their_non_pawn = self.non_pawn_correction_history[us][them]
            [board.non_pawn_key(!color) as usize & (CORR_SIZE - 1)]
            as i32;
        let continuation = if ply >= 1 {
            let prev = self.stack_moves[ply - 1];
            if prev.is_null() {
                0
            } else {
                self.continuation_correction_history
                    [piece_to_index(self.stack_pieces[ply - 1] as usize, prev.to_sq().index())]
                    as i32
            }
        } else {
            0
        };
        (pawn + minor + own_non_pawn + their_non_pawn + continuation / 2) / 128
    }

    fn syzygy_wdl_score(
        &mut self,
        board: &Board,
        depth: i32,
        ply: usize,
        excluded: Move,
    ) -> Option<i32> {
        if ply == 0 || !excluded.is_null() || !self.can_probe_syzygy(board, depth) {
            return None;
        }
        let wdl = syzygy::probe_wdl(board, self.syzygy_50_move_rule)?;
        self.tb_hits += 1;
        Some(self.score_from_syzygy_wdl(wdl, ply))
    }

    fn can_probe_syzygy(&self, board: &Board, depth: i32) -> bool {
        self.syzygy_largest > 0
            && depth >= self.syzygy_probe_depth
            && board.castling.0 == 0
            && board.occupied_count() as usize <= self.syzygy_largest
    }

    fn can_probe_syzygy_root(&self, board: &Board) -> bool {
        self.syzygy_largest > 0
            && board.castling.0 == 0
            && board.occupied_count() as usize <= self.syzygy_largest
    }

    fn score_from_syzygy_wdl(&self, wdl: Wdl, ply: usize) -> i32 {
        match wdl {
            Wdl::Win => TB_WIN_SCORE - ply as i32,
            Wdl::CursedWin if !self.syzygy_50_move_rule => TB_WIN_SCORE - ply as i32,
            Wdl::Loss => -TB_WIN_SCORE + ply as i32,
            Wdl::BlessedLoss if !self.syzygy_50_move_rule => -TB_WIN_SCORE + ply as i32,
            Wdl::BlessedLoss | Wdl::Draw | Wdl::CursedWin => 0,
        }
    }

    fn update_correction(&mut self, board: &Board, diff: i32, depth: i32, ply: usize) {
        let color = board.side_to_move();
        let us = color as usize;
        let them = (!color) as usize;
        let scaled = (diff * depth.max(1)).clamp(-1024, 1024);
        update_hist_entry(
            &mut self.correction_history[us][board.pawn_key() as usize & (CORR_SIZE - 1)],
            scaled,
            HISTORY_MAX,
        );
        update_hist_entry(
            &mut self.minor_correction_history[us][board.minor_key() as usize & (CORR_SIZE - 1)],
            scaled,
            HISTORY_MAX,
        );
        update_hist_entry(
            &mut self.non_pawn_correction_history[us][us]
                [board.non_pawn_key(color) as usize & (CORR_SIZE - 1)],
            scaled,
            HISTORY_MAX,
        );
        update_hist_entry(
            &mut self.non_pawn_correction_history[us][them]
                [board.non_pawn_key(!color) as usize & (CORR_SIZE - 1)],
            scaled,
            HISTORY_MAX,
        );
        if ply >= 1 {
            let prev = self.stack_moves[ply - 1];
            if !prev.is_null() {
                update_hist_entry(
                    &mut self.continuation_correction_history
                        [piece_to_index(self.stack_pieces[ply - 1] as usize, prev.to_sq().index())],
                    scaled / 2,
                    HISTORY_MAX,
                );
            }
        }
    }

    fn store_tt(
        &mut self,
        key: u64,
        depth: i32,
        score: i32,
        bound: Bound,
        mv: Move,
        ply: usize,
        static_eval: i32,
    ) {
        if self.tt_write_mode == TtWriteMode::Helper {
            let min_depth = match bound {
                Bound::Exact => 3,
                Bound::Lower => 5,
                Bound::Upper => 7,
            };
            if depth < min_depth {
                return;
            }
        }
        self.tt
            .store(key, depth, score, bound, mv, ply, static_eval);
    }

    fn check_stop<P: FnMut() -> SearchEvent + ?Sized>(&mut self, poll: &mut P) -> bool {
        self.nodes += 1;
        if self.limits.nodes > 0 && self.nodes >= self.limits.nodes {
            self.stopped = true;
            return true;
        }
        if self.nodes & 2047 == 0 {
            match poll() {
                SearchEvent::Quit => {
                    self.quit = true;
                    self.stopped = true;
                }
                SearchEvent::Stop => {
                    self.stopped = true;
                }
                SearchEvent::PonderHit => {
                    self.pondering = false;
                    self.ponderhit = true;
                    self.start = Instant::now();
                }
                SearchEvent::None => {}
            }
            if !self.pondering && self.elapsed_ms() >= self.limits.hard_ms {
                self.stopped = true;
            }
        }
        self.stopped
    }

    fn elapsed_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    fn send_info(&self, depth: usize, score: i32) {
        let pv = self.pv_table[0][..self.pv_len[0].min(MAX_PLY)]
            .iter()
            .copied()
            .filter(|mv| !mv.is_null())
            .collect::<Vec<_>>();
        self.send_info_line(depth, score, None, &pv);
    }

    fn send_info_line(&self, depth: usize, score: i32, multipv: Option<usize>, pv: &[Move]) {
        let elapsed_ms = self.start.elapsed().as_millis();
        let nps = if elapsed_ms > 0 {
            self.nodes as u128 * 1000 / elapsed_ms
        } else {
            self.nodes as u128
        };
        let pv = pv
            .iter()
            .map(|mv| mv.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(multipv) = multipv {
            println!(
                "info depth {} seldepth {} multipv {} score {} nodes {} nps {} hashfull {} tbhits {} time {} pv {}",
                depth,
                self.seldepth,
                multipv,
                format_score(score),
                self.nodes,
                nps,
                self.hashfull(),
                self.tb_hits,
                elapsed_ms,
                pv
            );
        } else {
            println!(
                "info depth {} seldepth {} score {} nodes {} nps {} hashfull {} tbhits {} time {} pv {}",
                depth,
                self.seldepth,
                format_score(score),
                self.nodes,
                nps,
                self.hashfull(),
                self.tb_hits,
                elapsed_ms,
                pv
            );
        }
    }

    fn set_root_pv(&mut self, pv: &[Move]) {
        self.pv_len[0] = pv.len().min(MAX_PLY);
        for (index, mv) in pv.iter().take(MAX_PLY).copied().enumerate() {
            self.pv_table[0][index] = mv;
        }
        for index in self.pv_len[0]..MAX_PLY {
            self.pv_table[0][index] = Move::NULL;
        }
    }

    fn result_for_no_legal_moves(&self, board: &Board) -> GameResult {
        if board.is_in_check() {
            match board.side_to_move() {
                Color::White => GameResult::BlackCheckmates,
                Color::Black => GameResult::WhiteCheckmates,
            }
        } else {
            GameResult::Stalemate
        }
    }
}

fn lmr_reduction(depth: i32, move_index: usize) -> i32 {
    if depth < 3 || move_index < 2 {
        return 0;
    }
    LMR_TABLE[depth.min(63) as usize][move_index.min(63)]
}

fn late_move_prune_count(depth: i32, improving: bool) -> usize {
    let base = 3 + depth * depth / 2;
    if improving {
        (base + depth) as usize
    } else {
        base as usize
    }
}

fn move_gives_check(board: &Board, mv: Move, cache: &mut Option<bool>) -> bool {
    match *cache {
        Some(gives_check) => gives_check,
        None => {
            let gives_check = board.gives_check(mv);
            *cache = Some(gives_check);
            gives_check
        }
    }
}

fn select_parallel_result(results: &[SearchResult], root_moves: &[Move]) -> Option<SearchResult> {
    let max_depth = results
        .iter()
        .filter(|result| is_root_result(result, root_moves))
        .map(|result| result.depth)
        .max()?;

    let mut votes: Vec<(Move, usize, bool, i32, usize)> = Vec::new();
    for (index, result) in results.iter().enumerate() {
        if result.depth != max_depth || !is_root_result(result, root_moves) {
            continue;
        }
        if let Some(vote) = votes
            .iter_mut()
            .find(|(mv, _, _, _, _)| *mv == result.bestmove)
        {
            vote.1 += 1;
            vote.2 |= index == 0;
            if result.score > vote.3 {
                vote.3 = result.score;
                vote.4 = index;
            }
        } else {
            votes.push((result.bestmove, 1, index == 0, result.score, index));
        }
    }

    votes
        .into_iter()
        .max_by_key(|(_, count, main_vote, score, _)| (*count, *main_vote, *score))
        .and_then(|(_, _, _, _, index)| results.get(index).cloned())
        .or_else(|| {
            results
                .iter()
                .filter(|result| is_root_result(result, root_moves))
                .max_by_key(|result| (result.depth, result.score))
                .cloned()
        })
}

fn is_root_result(result: &SearchResult, root_moves: &[Move]) -> bool {
    result.depth > 0 && root_moves.iter().any(|&mv| mv == result.bestmove)
}

fn format_score(score: i32) -> String {
    if score >= MATE_SCORE - MAX_PLY as i32 {
        format!("mate {}", (MATE_SCORE - score + 1) / 2)
    } else if score <= -MATE_SCORE + MAX_PLY as i32 {
        format!("mate -{}", (MATE_SCORE + score + 1) / 2)
    } else {
        format!("cp {score}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiescence_detects_mate_after_first_qply_check() {
        let mut searcher = Searcher::default();
        let mut board =
            Board::from_fen("rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3")
                .expect("valid fool's mate FEN");

        let score = searcher.quiescence(&mut board, -INF_SCORE, INF_SCORE, 0, 1, &mut || {
            SearchEvent::None
        });

        assert_eq!(score, -MATE_SCORE);
    }

    #[test]
    fn search_root_respects_restricted_root_moves() {
        let mut searcher = Searcher::default();
        let board = Board::default();
        let forced = board.parse_move("a2a3").expect("legal root move");
        let engine_options = EngineOptions::default();
        let limits = SearchLimits {
            depth: 1.0,
            ..SearchLimits::default()
        };
        searcher.reset_search_state(&limits, &engine_options, board.side_to_move(), true, true);

        let result = searcher.search_root(board, &[forced], false, &mut || SearchEvent::None);

        assert_eq!(result.bestmove, forced);
    }

    #[test]
    fn staged_picker_delays_bad_captures_until_after_quiets() {
        let searcher = Searcher::default();
        let mut board = Board::from_fen("4k3/8/4p3/3p4/8/2N5/8/4K3 w - - 0 1").expect("valid FEN");
        let losing_capture = board
            .parse_move("c3d5")
            .expect("knight capture must be legal");
        assert!(losing_capture.is_capture());
        assert!(!board.see_ge(losing_capture, 0));

        let mut picker = MovePicker::staged(&searcher, &mut board, Move::NULL, 0);
        let mut quiet_seen = false;
        let mut losing_capture_seen = false;

        while let Some(picked) = picker.next(&searcher, &mut board) {
            if picked.mv == losing_capture {
                assert!(
                    quiet_seen,
                    "losing captures should be staged after quiet moves"
                );
                losing_capture_seen = true;
                break;
            }
            if board.is_quiet_move(picked.mv) {
                quiet_seen = true;
            }
        }

        assert!(
            quiet_seen,
            "test position must have at least one quiet move"
        );
        assert!(
            losing_capture_seen,
            "test position must include the losing capture"
        );
    }
}
