use std::ffi::CString;
use std::fs;
use std::os::raw::{c_char, c_int, c_uint};
use std::sync::{
    LazyLock, Mutex,
    atomic::{AtomicUsize, Ordering},
};

use crate::board::{Board, Color, Move, Piece};

const TB_LOSS: u32 = 0;
const TB_BLESSED_LOSS: u32 = 1;
const TB_DRAW: u32 = 2;
const TB_CURSED_WIN: u32 = 3;
const TB_WIN: u32 = 4;
const TB_RESULT_FAILED: u32 = 0xFFFF_FFFF;
const TB_RESULT_WDL_MASK: u32 = 0x0000_000F;
const TB_RESULT_TO_MASK: u32 = 0x0000_03F0;
const TB_RESULT_FROM_MASK: u32 = 0x0000_FC00;
const TB_RESULT_PROMOTES_MASK: u32 = 0x0007_0000;
const TB_RESULT_WDL_SHIFT: u32 = 0;
const TB_RESULT_TO_SHIFT: u32 = 4;
const TB_RESULT_FROM_SHIFT: u32 = 10;
const TB_RESULT_PROMOTES_SHIFT: u32 = 16;
const TB_MAX_MOVES: usize = 193;
const TB_MAX_PLY: usize = 256;

static SYZYGY_PATH: LazyLock<Mutex<String>> = LazyLock::new(|| Mutex::new(String::new()));
static LARGEST: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    static mut TB_LARGEST: c_uint;

    fn tb_init(path: *const c_char) -> bool;
    fn tb_probe_wdl_impl(
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        ep: c_uint,
        turn: bool,
    ) -> c_uint;
    fn tb_probe_root_impl(
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        rule50: c_uint,
        ep: c_uint,
        turn: bool,
        results: *mut c_uint,
    ) -> c_uint;
    fn tb_probe_root_dtz(
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        rule50: c_uint,
        castling: c_uint,
        ep: c_uint,
        turn: bool,
        has_repeated: bool,
        use_rule50: bool,
        results: *mut TbRootMovesRaw,
    ) -> c_int;
    fn tb_probe_root_wdl(
        white: u64,
        black: u64,
        kings: u64,
        queens: u64,
        rooks: u64,
        bishops: u64,
        knights: u64,
        pawns: u64,
        rule50: c_uint,
        castling: c_uint,
        ep: c_uint,
        turn: bool,
        use_rule50: bool,
        results: *mut TbRootMovesRaw,
    ) -> c_int;
}

#[repr(C)]
#[derive(Copy, Clone)]
struct TbRootMoveRaw {
    mv: u16,
    pv: [u16; TB_MAX_PLY],
    pv_size: c_uint,
    tb_score: i32,
    tb_rank: i32,
}

impl Default for TbRootMoveRaw {
    fn default() -> Self {
        Self {
            mv: 0,
            pv: [0; TB_MAX_PLY],
            pv_size: 0,
            tb_score: 0,
            tb_rank: 0,
        }
    }
}

#[repr(C)]
struct TbRootMovesRaw {
    size: c_uint,
    moves: [TbRootMoveRaw; TB_MAX_MOVES],
}

impl Default for TbRootMovesRaw {
    fn default() -> Self {
        Self {
            size: 0,
            moves: [TbRootMoveRaw::default(); TB_MAX_MOVES],
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Wdl {
    Loss,
    BlessedLoss,
    Draw,
    CursedWin,
    Win,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RootMove {
    pub from: u8,
    pub to: u8,
    pub promotes: Option<Piece>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RootProbe {
    pub wdl: Wdl,
    pub best_move: Option<RootMove>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RootMoveProbe {
    pub root_move: RootMove,
    pub rank: i32,
    pub score: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootMoveProbes {
    pub used_dtz: bool,
    pub moves: Vec<RootMoveProbe>,
}

#[derive(Copy, Clone)]
struct TbPosition {
    white: u64,
    black: u64,
    kings: u64,
    queens: u64,
    rooks: u64,
    bishops: u64,
    knights: u64,
    pawns: u64,
    rule50: u32,
    ep: u32,
    turn: bool,
}

pub fn initialize(path: &str) -> usize {
    let mut current_path = SYZYGY_PATH.lock().expect("syzygy path mutex poisoned");
    if *current_path == path {
        return largest();
    }

    if path.is_empty() {
        let empty = CString::new("").expect("empty string has no NUL");
        unsafe {
            tb_init(empty.as_ptr());
        }
        *current_path = String::new();
        LARGEST.store(0, Ordering::Relaxed);
        return 0;
    }

    let Ok(c_path) = CString::new(path) else {
        *current_path = String::new();
        LARGEST.store(0, Ordering::Relaxed);
        return 0;
    };

    let ok = unsafe { tb_init(c_path.as_ptr()) };
    let largest = if ok {
        unsafe { TB_LARGEST as usize }
    } else {
        0
    };
    *current_path = if ok { path.to_string() } else { String::new() };
    LARGEST.store(largest, Ordering::Relaxed);
    largest
}

pub fn current_path() -> String {
    SYZYGY_PATH
        .lock()
        .expect("syzygy path mutex poisoned")
        .clone()
}

#[inline(always)]
pub fn largest() -> usize {
    LARGEST.load(Ordering::Relaxed)
}

pub fn tablebase_file_counts(path: &str) -> (usize, usize) {
    let mut wdl = 0usize;
    let mut dtz = 0usize;
    for part in path
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let Ok(entries) = fs::read_dir(part) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            match extension.to_ascii_lowercase().as_str() {
                "rtbw" => wdl += 1,
                "rtbz" => dtz += 1,
                _ => {}
            }
        }
    }
    (wdl, dtz)
}

pub fn probe_wdl(board: &Board, use_rule50: bool) -> Option<Wdl> {
    if !can_probe(board, use_rule50, false) {
        return None;
    }

    let pos = tb_position(board);
    let result = unsafe {
        tb_probe_wdl_impl(
            pos.white,
            pos.black,
            pos.kings,
            pos.queens,
            pos.rooks,
            pos.bishops,
            pos.knights,
            pos.pawns,
            pos.ep,
            pos.turn,
        )
    };
    wdl_from_raw(result)
}

pub fn probe_root(board: &Board, use_rule50: bool) -> Option<RootProbe> {
    if !can_probe(board, use_rule50, true) {
        return None;
    }

    let pos = tb_position(board);
    let result = unsafe {
        tb_probe_root_impl(
            pos.white,
            pos.black,
            pos.kings,
            pos.queens,
            pos.rooks,
            pos.bishops,
            pos.knights,
            pos.pawns,
            pos.rule50,
            pos.ep,
            pos.turn,
            std::ptr::null_mut(),
        )
    };
    if result == TB_RESULT_FAILED {
        return None;
    }

    let wdl = wdl_from_raw((result & TB_RESULT_WDL_MASK) >> TB_RESULT_WDL_SHIFT)?;
    Some(RootProbe {
        wdl,
        best_move: root_move_from_result(result),
    })
}

pub fn probe_root_moves(
    board: &Board,
    use_rule50: bool,
    has_repeated: bool,
) -> Option<RootMoveProbes> {
    if !can_probe(board, use_rule50, true) {
        return None;
    }

    let pos = tb_position(board);
    let mut results = TbRootMovesRaw::default();
    let dtz_ok = unsafe {
        tb_probe_root_dtz(
            pos.white,
            pos.black,
            pos.kings,
            pos.queens,
            pos.rooks,
            pos.bishops,
            pos.knights,
            pos.pawns,
            pos.rule50,
            0,
            pos.ep,
            pos.turn,
            has_repeated,
            use_rule50,
            &mut results,
        )
    };
    let used_dtz = dtz_ok != 0;
    if !used_dtz {
        results = TbRootMovesRaw::default();
        let wdl_ok = unsafe {
            tb_probe_root_wdl(
                pos.white,
                pos.black,
                pos.kings,
                pos.queens,
                pos.rooks,
                pos.bishops,
                pos.knights,
                pos.pawns,
                pos.rule50,
                0,
                pos.ep,
                pos.turn,
                use_rule50,
                &mut results,
            )
        };
        if wdl_ok == 0 {
            return None;
        }
    }

    let len = (results.size as usize).min(TB_MAX_MOVES);
    let mut moves = Vec::new();
    for result in results.moves.iter().take(len) {
        moves.push(RootMoveProbe {
            root_move: root_move_from_tb_move(result.mv)?,
            rank: result.tb_rank,
            score: result.tb_score,
        });
    }

    if moves.is_empty() {
        None
    } else {
        Some(RootMoveProbes { used_dtz, moves })
    }
}

pub fn legal_move_from_root_probe(board: &Board, root_move: RootMove) -> Option<Move> {
    board.generate_legal_movelist().iter().copied().find(|mv| {
        mv.from_sq().0 == root_move.from
            && mv.to_sq().0 == root_move.to
            && mv.promotion() == root_move.promotes
    })
}

fn can_probe(board: &Board, use_rule50: bool, root: bool) -> bool {
    if largest() == 0 || board.castling.0 != 0 {
        return false;
    }
    if use_rule50 && !root && board.halfmove_clock != 0 {
        return false;
    }
    board.occupied_count() as usize <= largest()
}

fn tb_position(board: &Board) -> TbPosition {
    TbPosition {
        white: board.color_occ(Color::White).0,
        black: board.color_occ(Color::Black).0,
        kings: (board.pieces(Color::White, Piece::King) | board.pieces(Color::Black, Piece::King))
            .0,
        queens: (board.pieces(Color::White, Piece::Queen)
            | board.pieces(Color::Black, Piece::Queen))
        .0,
        rooks: (board.pieces(Color::White, Piece::Rook) | board.pieces(Color::Black, Piece::Rook))
            .0,
        bishops: (board.pieces(Color::White, Piece::Bishop)
            | board.pieces(Color::Black, Piece::Bishop))
        .0,
        knights: (board.pieces(Color::White, Piece::Knight)
            | board.pieces(Color::Black, Piece::Knight))
        .0,
        pawns: (board.pieces(Color::White, Piece::Pawn) | board.pieces(Color::Black, Piece::Pawn))
            .0,
        rule50: board.halfmove_clock as u32,
        ep: board.ep_square().map_or(0, |sq| sq.0 as u32),
        turn: board.side_to_move() == Color::White,
    }
}

fn root_move_from_result(result: u32) -> Option<RootMove> {
    let from = ((result & TB_RESULT_FROM_MASK) >> TB_RESULT_FROM_SHIFT) as u8;
    let to = ((result & TB_RESULT_TO_MASK) >> TB_RESULT_TO_SHIFT) as u8;
    let promotes = match (result & TB_RESULT_PROMOTES_MASK) >> TB_RESULT_PROMOTES_SHIFT {
        0 => None,
        1 => Some(Piece::Queen),
        2 => Some(Piece::Rook),
        3 => Some(Piece::Bishop),
        4 => Some(Piece::Knight),
        _ => return None,
    };
    if from == to {
        None
    } else {
        Some(RootMove { from, to, promotes })
    }
}

fn root_move_from_tb_move(mv: u16) -> Option<RootMove> {
    let from = ((mv >> 6) & 0x3F) as u8;
    let to = (mv & 0x3F) as u8;
    let promotes = match (mv >> 12) & 0x7 {
        0 => None,
        1 => Some(Piece::Queen),
        2 => Some(Piece::Rook),
        3 => Some(Piece::Bishop),
        4 => Some(Piece::Knight),
        _ => return None,
    };
    if from == to {
        None
    } else {
        Some(RootMove { from, to, promotes })
    }
}

fn wdl_from_raw(value: u32) -> Option<Wdl> {
    match value {
        TB_LOSS => Some(Wdl::Loss),
        TB_BLESSED_LOSS => Some(Wdl::BlessedLoss),
        TB_DRAW => Some(Wdl::Draw),
        TB_CURSED_WIN => Some(Wdl::CursedWin),
        TB_WIN => Some(Wdl::Win),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::board::Square;
    use std::fs;
    use std::path::Path;
    use std::sync::{LazyLock, Mutex};

    static TEST_SYZYGY_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn encoded_root_result(from: Square, to: Square, promotes: u32) -> u32 {
        (TB_WIN << TB_RESULT_WDL_SHIFT)
            | ((to.0 as u32) << TB_RESULT_TO_SHIFT)
            | ((from.0 as u32) << TB_RESULT_FROM_SHIFT)
            | (promotes << TB_RESULT_PROMOTES_SHIFT)
    }

    fn encoded_tb_move(from: Square, to: Square, promotes: u16) -> u16 {
        ((from.0 as u16) << 6) | to.0 as u16 | (promotes << 12)
    }

    #[test]
    fn wdl_from_raw_maps_fathom_values() {
        assert_eq!(wdl_from_raw(TB_LOSS), Some(Wdl::Loss));
        assert_eq!(wdl_from_raw(TB_BLESSED_LOSS), Some(Wdl::BlessedLoss));
        assert_eq!(wdl_from_raw(TB_DRAW), Some(Wdl::Draw));
        assert_eq!(wdl_from_raw(TB_CURSED_WIN), Some(Wdl::CursedWin));
        assert_eq!(wdl_from_raw(TB_WIN), Some(Wdl::Win));
        assert_eq!(wdl_from_raw(TB_RESULT_FAILED), None);
        assert_eq!(wdl_from_raw(99), None);
    }

    #[test]
    fn root_move_from_result_decodes_square_and_promotion_fields() {
        assert_eq!(
            root_move_from_result(encoded_root_result(Square::A7, Square::A8, 1)),
            Some(RootMove {
                from: Square::A7.0,
                to: Square::A8.0,
                promotes: Some(Piece::Queen),
            })
        );
        assert_eq!(
            root_move_from_result(encoded_root_result(Square::B2, Square::B1, 4)),
            Some(RootMove {
                from: Square::B2.0,
                to: Square::B1.0,
                promotes: Some(Piece::Knight),
            })
        );
        assert_eq!(
            root_move_from_result(encoded_root_result(Square::E2, Square::E4, 0)),
            Some(RootMove {
                from: Square::E2.0,
                to: Square::E4.0,
                promotes: None,
            })
        );
    }

    #[test]
    fn root_move_from_result_rejects_no_move_and_unknown_promotion() {
        assert_eq!(
            root_move_from_result(encoded_root_result(Square::E2, Square::E2, 0)),
            None
        );
        assert_eq!(
            root_move_from_result(encoded_root_result(Square::A7, Square::A8, 7)),
            None
        );
    }

    #[test]
    fn root_move_from_tb_move_decodes_fathom_move_fields() {
        assert_eq!(
            root_move_from_tb_move(encoded_tb_move(Square::C6, Square::C7, 0)),
            Some(RootMove {
                from: Square::C6.0,
                to: Square::C7.0,
                promotes: None,
            })
        );
        assert_eq!(
            root_move_from_tb_move(encoded_tb_move(Square::A7, Square::A8, 1)),
            Some(RootMove {
                from: Square::A7.0,
                to: Square::A8.0,
                promotes: Some(Piece::Queen),
            })
        );
        assert_eq!(
            root_move_from_tb_move(encoded_tb_move(Square::A7, Square::A8, 7)),
            None
        );
    }

    #[test]
    fn tablebase_file_counts_scan_semicolon_separated_paths() {
        let base =
            std::env::temp_dir().join(format!("rarog-syzygy-counts-{}-{}", std::process::id(), 1));
        let first = base.join("a");
        let second = base.join("b");
        fs::create_dir_all(&first).expect("create first temp tablebase directory");
        fs::create_dir_all(&second).expect("create second temp tablebase directory");
        fs::write(first.join("KQvK.rtbw"), []).expect("write WDL file");
        fs::write(first.join("KQvK.rtbz"), []).expect("write DTZ file");
        fs::write(second.join("KRvK.RTBW"), []).expect("write upper-case WDL file");
        fs::write(second.join("README.txt"), []).expect("write ignored file");

        let path = format!("{};{}", first.display(), second.display());
        assert_eq!(tablebase_file_counts(&path), (2, 1));

        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn probe_root_moves_uses_local_syzygy_tables_when_available() {
        let _guard = TEST_SYZYGY_LOCK.lock().expect("syzygy test lock poisoned");
        let path = "D:\\chess\\Syzygy345";
        if !Path::new(path).join("KQvK.rtbw").exists()
            || !Path::new(path).join("KQvK.rtbz").exists()
        {
            return;
        }

        assert!(initialize(path) >= 3);
        let board = Board::from_fen("k7/8/2KQ4/8/8/8/8/8 w - - 0 1").expect("valid FEN");
        let root = probe_root_moves(&board, true, false).expect("root TB probe succeeds");

        assert!(root.used_dtz);
        assert!(
            root.moves.iter().any(|probe| {
                legal_move_from_root_probe(&board, probe.root_move)
                    .is_some_and(|mv| mv.to_string() == "c6c7")
            }),
            "expected KQvK root probe to include c6c7"
        );

        initialize("");
    }

    #[test]
    fn tb_position_exports_bitboards_side_ep_and_rule50_state() {
        let board = Board::from_fen("4k3/8/8/8/4Pp2/8/8/4K3 b - e3 7 42").expect("valid FEN");

        let pos = tb_position(&board);

        assert_eq!(pos.white, board.color_occ(Color::White).0);
        assert_eq!(pos.black, board.color_occ(Color::Black).0);
        assert_eq!(
            pos.kings,
            (board.pieces(Color::White, Piece::King) | board.pieces(Color::Black, Piece::King)).0
        );
        assert_eq!(
            pos.pawns,
            (board.pieces(Color::White, Piece::Pawn) | board.pieces(Color::Black, Piece::Pawn)).0
        );
        assert_eq!(pos.rule50, 7);
        assert_eq!(pos.ep, Square::E3.0 as u32);
        assert!(!pos.turn);
    }

    #[test]
    fn legal_move_from_root_probe_requires_exact_promotion_match() {
        let board = Board::from_fen("4k3/P7/8/8/8/8/8/4K3 w - - 0 1").expect("valid FEN");

        let queen_promo = legal_move_from_root_probe(
            &board,
            RootMove {
                from: Square::A7.0,
                to: Square::A8.0,
                promotes: Some(Piece::Queen),
            },
        )
        .expect("queen promotion must be legal");

        assert_eq!(queen_promo.to_string(), "a7a8q");
        assert!(
            legal_move_from_root_probe(
                &board,
                RootMove {
                    from: Square::A7.0,
                    to: Square::A8.0,
                    promotes: None,
                },
            )
            .is_none()
        );
    }

    #[test]
    fn initialize_rejects_path_with_nul_without_calling_fathom() {
        let _guard = TEST_SYZYGY_LOCK.lock().expect("syzygy test lock poisoned");
        assert_eq!(initialize("bad\0path"), 0);
        assert_eq!(largest(), 0);
    }
}
