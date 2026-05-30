/// Board representation.
///
/// Uses 12 bitboards (one per color×piece), two occupancy bitboards, and an
/// incremental Zobrist hash.  Make/unmake are performed in-place with an
/// internal history stack — no full-struct copies needed.
use std::fmt;

use super::attacks::ATTACKS;
use super::bitboard::Bitboard;
use super::movegen::generate_legal_moves;
use super::moves::{
    CAPTURE, CASTLE_KINGSIDE, CASTLE_QUEENSIDE, DOUBLE_PUSH, EN_PASSANT, Move, MoveList,
    PROMO_BISHOP, PROMO_CAPTURE_BISHOP, PROMO_CAPTURE_KNIGHT, PROMO_CAPTURE_QUEEN,
    PROMO_CAPTURE_ROOK, PROMO_KNIGHT, PROMO_QUEEN, PROMO_ROOK, QUIET,
};
use super::piece::{CastlingRights, Color, Piece};
use super::square::{Rank, Square};
use super::zobrist::ZOBRIST;

// -----------------------------------------------------------------------
// Unmake info — everything needed to undo a move
// -----------------------------------------------------------------------

#[derive(Copy, Clone)]
struct UnmakeInfo {
    /// Captured piece, if any.  255 = no capture.
    captured: u8, // piece index: color*6 + piece, or 255
    castling: CastlingRights,
    ep_sq: u8, // 255 = no EP
    halfmove_clock: u8,
    fullmove: u16,
    hash: u64,
    checkers: Bitboard,
    piece_threats: [Bitboard; 6],
    all_threats: Bitboard,
}

const NO_PIECE: u8 = 255;
const PIECE_FROM_ENCODED: [Piece; 12] = [
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
    Piece::King,
    Piece::Pawn,
    Piece::Knight,
    Piece::Bishop,
    Piece::Rook,
    Piece::Queen,
    Piece::King,
];

// -----------------------------------------------------------------------
// Board
// -----------------------------------------------------------------------

pub const STARTING_FEN: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum GameResult {
    WhiteCheckmates,
    BlackCheckmates,
    Stalemate,
    Draw,
}

/// The chess board.  Square A1 = 0, H8 = 63 (rank-major, little-endian).
pub struct Board {
    /// `pieces[color * 6 + piece_type]`
    pieces: [Bitboard; 12],
    /// `occupancy[color]`
    occupancy: [Bitboard; 2],
    /// Union of both occupancy bitboards.
    pub all_occ: Bitboard,
    /// Encoded piece on each square, or 255 for empty.
    mailbox: [u8; 64],
    /// Side to move.
    pub side_to_move: Color,
    pub castling: CastlingRights,
    /// En passant target square (the square a capturing pawn moves *to*).
    /// `255` encodes "no EP".
    ep_sq: u8,
    pub halfmove_clock: u8,
    pub fullmove: u16,
    /// Incrementally updated Zobrist hash.
    pub hash: u64,
    pawn_hash: u64,
    minor_hash: u64,
    non_pawn_hash: [u64; 2],
    checkers: Bitboard,
    piece_threats: [Bitboard; 6],
    all_threats: Bitboard,
    history: Vec<UnmakeInfo>,
}

impl Clone for Board {
    fn clone(&self) -> Self {
        let mut history = Vec::with_capacity(self.history.capacity().max(128));
        history.extend_from_slice(&self.history);
        Self {
            pieces: self.pieces,
            occupancy: self.occupancy,
            all_occ: self.all_occ,
            mailbox: self.mailbox,
            side_to_move: self.side_to_move,
            castling: self.castling,
            ep_sq: self.ep_sq,
            halfmove_clock: self.halfmove_clock,
            fullmove: self.fullmove,
            hash: self.hash,
            pawn_hash: self.pawn_hash,
            minor_hash: self.minor_hash,
            non_pawn_hash: self.non_pawn_hash,
            checkers: self.checkers,
            piece_threats: self.piece_threats,
            all_threats: self.all_threats,
            history,
        }
    }
}

impl Board {
    // -----------------------------------------------------------------------
    // Construction
    // -----------------------------------------------------------------------

    pub fn starting_position() -> Self {
        Self::from_fen(STARTING_FEN).expect("starting FEN is valid")
    }

    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let mut board = Self {
            pieces: [Bitboard::EMPTY; 12],
            occupancy: [Bitboard::EMPTY; 2],
            all_occ: Bitboard::EMPTY,
            mailbox: [NO_PIECE; 64],
            side_to_move: Color::White,
            castling: CastlingRights::NONE,
            ep_sq: 255,
            halfmove_clock: 0,
            fullmove: 1,
            hash: 0,
            pawn_hash: 0,
            minor_hash: 0,
            non_pawn_hash: [0; 2],
            checkers: Bitboard::EMPTY,
            piece_threats: [Bitboard::EMPTY; 6],
            all_threats: Bitboard::EMPTY,
            history: Vec::with_capacity(128),
        };

        let parts = fen.split_whitespace().collect::<Vec<_>>();
        if !(4..=6).contains(&parts.len()) {
            return Err("FEN must contain 4 to 6 fields".to_string());
        }

        // 1. Piece placement
        let placement = parts[0];
        let mut rank = 7u8;
        let mut file = 0u8;
        for ch in placement.chars() {
            match ch {
                '/' => {
                    if file != 8 {
                        return Err(format!("incomplete FEN rank before '/' in {placement}"));
                    }
                    if rank == 0 {
                        return Err(format!("too many FEN ranks in {placement}"));
                    }
                    rank -= 1;
                    file = 0;
                }
                '1'..='8' => {
                    file += ch as u8 - b'0';
                    if file > 8 {
                        return Err(format!("too many squares in FEN rank {rank}"));
                    }
                }
                c => {
                    let (color, piece) = fen_char_to_piece(c)?;
                    if file >= 8 {
                        return Err(format!("too many squares in FEN rank {rank}"));
                    }
                    if piece == Piece::Pawn && (rank == 0 || rank == 7) {
                        return Err("pawns are not legal on the first or eighth rank".to_string());
                    }
                    let sq = Square(rank * 8 + file);
                    board.add_piece(color, piece, sq);
                    board.hash ^= ZOBRIST.piece(color, piece, sq);
                    file += 1;
                }
            }
        }
        if rank != 0 || file != 8 {
            return Err(format!(
                "piece placement must contain 8 complete ranks: {placement}"
            ));
        }

        // 2. Side to move
        match parts[1] {
            "w" => board.side_to_move = Color::White,
            "b" => {
                board.side_to_move = Color::Black;
                board.hash ^= ZOBRIST.side();
            }
            s => return Err(format!("invalid side to move: {s}")),
        }

        // 3. Castling rights
        let castling_str = parts[2];
        let mut cr = CastlingRights::NONE;
        if castling_str.contains('-') && castling_str.len() > 1 {
            return Err(format!("invalid castling rights: {castling_str}"));
        }
        for c in castling_str.chars() {
            match c {
                'K' if !cr.has(CastlingRights::WHITE_KINGSIDE) => {
                    cr.0 |= CastlingRights::WHITE_KINGSIDE.0
                }
                'Q' if !cr.has(CastlingRights::WHITE_QUEENSIDE) => {
                    cr.0 |= CastlingRights::WHITE_QUEENSIDE.0
                }
                'k' if !cr.has(CastlingRights::BLACK_KINGSIDE) => {
                    cr.0 |= CastlingRights::BLACK_KINGSIDE.0
                }
                'q' if !cr.has(CastlingRights::BLACK_QUEENSIDE) => {
                    cr.0 |= CastlingRights::BLACK_QUEENSIDE.0
                }
                '-' => {}
                c => return Err(format!("invalid castling char: {c}")),
            }
        }
        validate_castling_rights(&board, cr)?;
        board.castling = cr;
        board.hash ^= ZOBRIST.castling(cr);

        // 4. En passant
        let ep_str = parts[3];
        let ep_candidate = if ep_str != "-" {
            let sq = Square::from_algebraic(ep_str)
                .ok_or_else(|| format!("invalid ep square: {ep_str}"))?;
            Some(sq)
        } else {
            None
        };

        // 5. Halfmove clock
        if let Some(s) = parts.get(4) {
            board.halfmove_clock = s
                .parse::<u8>()
                .map_err(|_| format!("invalid halfmove clock: {s}"))?;
        }

        // 6. Fullmove number
        if let Some(s) = parts.get(5) {
            board.fullmove = s
                .parse::<u16>()
                .map_err(|_| format!("invalid fullmove number: {s}"))?
                .max(1);
        }

        board.validate_position()?;
        if let Some(ep_sq) = ep_candidate {
            board.set_legal_ep_square(ep_sq)?;
        }
        board.checkers = board.calculate_checkers();
        board.update_threats();
        Ok(board)
    }

    /// Serialize the board to a FEN string.
    pub fn to_fen(&self) -> String {
        let mut fen = String::with_capacity(80);

        // Piece placement (rank 8 down to rank 1)
        for rank in (0..8).rev() {
            let mut empty = 0u8;
            for file in 0..8u8 {
                let sq = Square(rank * 8 + file);
                if let Some((color, piece)) = self.piece_at(sq) {
                    if empty > 0 {
                        fen.push((b'0' + empty) as char);
                        empty = 0;
                    }
                    let c = match piece {
                        Piece::Pawn => 'p',
                        Piece::Knight => 'n',
                        Piece::Bishop => 'b',
                        Piece::Rook => 'r',
                        Piece::Queen => 'q',
                        Piece::King => 'k',
                    };
                    fen.push(if color == Color::White {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    });
                } else {
                    empty += 1;
                }
            }
            if empty > 0 {
                fen.push((b'0' + empty) as char);
            }
            if rank > 0 {
                fen.push('/');
            }
        }

        fen.push(' ');
        fen.push(if self.side_to_move == Color::White {
            'w'
        } else {
            'b'
        });
        fen.push(' ');
        fen.push_str(self.castling.as_str());
        fen.push(' ');
        if self.ep_sq == 255 {
            fen.push('-');
        } else {
            fen.push_str(&Square(self.ep_sq).to_string());
        }
        fen.push(' ');
        fen.push_str(&self.halfmove_clock.to_string());
        fen.push(' ');
        fen.push_str(&self.fullmove.to_string());
        fen
    }

    // -----------------------------------------------------------------------
    // Piece accessors
    // -----------------------------------------------------------------------

    /// Bitboard for a specific color + piece type.
    #[inline(always)]
    pub fn pieces(&self, color: Color, piece: Piece) -> Bitboard {
        self.pieces[color as usize * 6 + piece as usize]
    }

    /// Bitboard for all pieces of a given color.
    #[inline(always)]
    pub fn color_occ(&self, color: Color) -> Bitboard {
        self.occupancy[color as usize]
    }

    /// Piece type and color at a given square, or `None` if empty.
    #[inline(always)]
    pub fn piece_at(&self, sq: Square) -> Option<(Color, Piece)> {
        decode_piece(self.mailbox[sq.index()])
    }

    /// Piece type only at a given square.
    #[inline(always)]
    pub fn piece_type_at(&self, sq: Square) -> Option<Piece> {
        decode_piece_type(self.mailbox[sq.index()])
    }

    /// King square for a given color.
    #[inline(always)]
    pub fn king_sq(&self, color: Color) -> Square {
        self.pieces(color, Piece::King).lsb()
    }

    /// En passant target square, if any.
    #[inline(always)]
    pub fn ep_square(&self) -> Option<Square> {
        if self.ep_sq == 255 {
            None
        } else {
            Some(Square(self.ep_sq))
        }
    }

    #[inline(always)]
    pub fn side_to_move(&self) -> Color {
        self.side_to_move
    }

    #[inline(always)]
    pub fn occupied_count(&self) -> u32 {
        self.all_occ.count()
    }

    #[inline(always)]
    pub fn occupied(&self) -> Bitboard {
        self.all_occ
    }

    #[inline(always)]
    pub fn all_threats(&self) -> Bitboard {
        self.all_threats
    }

    #[inline(always)]
    pub fn piece_threats(&self, piece: Piece) -> Bitboard {
        self.piece_threats[piece as usize]
    }

    #[inline(always)]
    pub fn piece_on(&self, sq: Square) -> Option<Piece> {
        self.piece_type_at(sq)
    }

    #[inline(always)]
    pub fn color_on(&self, sq: Square) -> Option<Color> {
        self.piece_at(sq).map(|(color, _)| color)
    }

    #[inline(always)]
    pub fn moving_piece(&self, mv: Move) -> Piece {
        debug_assert!(self.mailbox[mv.from_sq().index()] < 12);
        self.piece_type_at_unchecked(mv.from_sq())
    }

    #[inline(always)]
    pub fn is_quiet_move(&self, mv: Move) -> bool {
        !mv.is_capture() && !mv.is_promo() && !mv.is_castling()
    }

    #[inline(always)]
    pub fn en_passant(&self) -> Option<Square> {
        self.ep_square()
    }

    pub fn parse_move(&self, input: &str) -> Option<Move> {
        self.legal_move(Move::from_uci(input)?)
    }

    pub fn pseudo_legal_move(&self, mv: Move) -> Option<Move> {
        if mv.is_null() {
            return None;
        }

        let from = mv.from_sq();
        let to = mv.to_sq();
        let us = self.side_to_move;
        let them = !us;
        let (color, piece) = self.piece_at(from)?;
        if color != us {
            return None;
        }
        match self.piece_at(to) {
            Some((target_color, _)) if target_color == us => return None,
            Some((_, Piece::King)) => return None,
            _ => {}
        }

        let promotion = mv.promotion();
        let atk = &*ATTACKS;
        let to_bb = Bitboard::from(to);
        let canonical = match piece {
            Piece::Pawn => {
                let delta = to.0 as i16 - from.0 as i16;
                let forward = if us == Color::White { 8 } else { -8 };
                let start_rank = if us == Color::White {
                    Rank::R2
                } else {
                    Rank::R7
                };
                let promotion_rank = if us == Color::White {
                    Rank::R8
                } else {
                    Rank::R1
                };
                let reaches_promotion = to.rank() == promotion_rank;
                if reaches_promotion != promotion.is_some() {
                    return None;
                }

                let is_pawn_attack = (atk.pawn(us, from) & to_bb).any();
                if is_pawn_attack {
                    if self.color_on(to) == Some(them) {
                        pawn_move_with_promotion(from, to, true, promotion)
                    } else if self.ep_square() == Some(to) && self.color_on(to).is_none() {
                        let cap_sq = if us == Color::White {
                            Square(to.0 - 8)
                        } else {
                            Square(to.0 + 8)
                        };
                        (self.piece_at(cap_sq) == Some((them, Piece::Pawn)))
                            .then_some(Move::new(from, to, EN_PASSANT))
                    } else {
                        None
                    }
                } else if delta == forward && self.color_on(to).is_none() {
                    pawn_move_with_promotion(from, to, false, promotion)
                } else if delta == 2 * forward
                    && from.rank() == start_rank
                    && self.color_on(to).is_none()
                {
                    let mid = Square((from.0 as i16 + forward) as u8);
                    (self.color_on(mid).is_none()).then_some(Move::new(from, to, DOUBLE_PUSH))
                } else {
                    None
                }
            }
            Piece::Knight => {
                if promotion.is_some() || (atk.knight(from) & to_bb).is_empty() {
                    None
                } else {
                    Some(Move::new(
                        from,
                        to,
                        capture_or_quiet(self.color_on(to), them),
                    ))
                }
            }
            Piece::Bishop => {
                if promotion.is_some() || (atk.bishop(from, self.all_occ) & to_bb).is_empty() {
                    None
                } else {
                    Some(Move::new(
                        from,
                        to,
                        capture_or_quiet(self.color_on(to), them),
                    ))
                }
            }
            Piece::Rook => {
                if promotion.is_some() || (atk.rook(from, self.all_occ) & to_bb).is_empty() {
                    None
                } else {
                    Some(Move::new(
                        from,
                        to,
                        capture_or_quiet(self.color_on(to), them),
                    ))
                }
            }
            Piece::Queen => {
                if promotion.is_some() || (atk.queen(from, self.all_occ) & to_bb).is_empty() {
                    None
                } else {
                    Some(Move::new(
                        from,
                        to,
                        capture_or_quiet(self.color_on(to), them),
                    ))
                }
            }
            Piece::King => {
                if promotion.is_some() {
                    None
                } else if (atk.king(from) & to_bb).any() {
                    Some(Move::new(
                        from,
                        to,
                        capture_or_quiet(self.color_on(to), them),
                    ))
                } else {
                    self.castling_move(from, to, us, them)
                }
            }
        }?;

        Some(canonical)
    }

    #[inline(always)]
    pub fn is_pseudo_legal(&self, mv: Move) -> bool {
        self.pseudo_legal_move(mv).is_some()
    }

    pub fn legal_move(&self, mv: Move) -> Option<Move> {
        let canonical = self.pseudo_legal_move(mv)?;
        self.king_safe_after(canonical).then_some(canonical)
    }

    #[inline(always)]
    pub fn is_legal(&self, mv: Move) -> bool {
        self.legal_move(mv).is_some()
    }

    pub fn play_uci(&mut self, input: &str) -> bool {
        if let Some(mv) = self.parse_move(input) {
            self.make_move(mv);
            true
        } else {
            false
        }
    }

    #[inline(always)]
    pub fn make_move_unchecked(&mut self, mv: Move) {
        self.make_move(mv);
    }

    pub fn generate_legal_moves(&self) -> Vec<Move> {
        generate_legal_moves(self)
    }

    pub fn generate_legal_movelist(&self) -> MoveList {
        super::movegen::generate_legal_movelist(self)
    }

    pub fn generate_legal_captures(&mut self) -> MoveList {
        super::movegen::generate_captures(self)
    }

    pub fn generate_legal_quiets(&self) -> MoveList {
        super::movegen::generate_quiets(self)
    }

    pub fn perft(&mut self, depth: u32) -> u64 {
        super::movegen::perft(self, depth)
    }

    pub fn captured_piece(&self, mv: Move) -> Option<Piece> {
        if mv.is_en_passant() {
            Some(Piece::Pawn)
        } else if mv.is_capture() {
            debug_assert!(self.mailbox[mv.to_sq().index()] < 12);
            Some(self.piece_type_at_unchecked(mv.to_sq()))
        } else {
            None
        }
    }

    #[inline(always)]
    pub fn is_capture(&self, mv: Move) -> bool {
        mv.is_capture()
    }

    #[inline(always)]
    pub fn is_en_passant(&self, mv: Move) -> bool {
        mv.is_en_passant()
    }

    pub fn gives_check(&self, mv: Move) -> bool {
        if mv.is_castling() {
            let mut board = self.clone();
            board.make_move(mv);
            return board.is_in_check();
        }

        let us = self.side_to_move;
        let them = !us;
        let from = mv.from_sq();
        let to = mv.to_sq();
        let from_bb = Bitboard::from(from);
        let to_bb = Bitboard::from(to);
        let their_king = self.king_sq(them);
        let their_king_bb = Bitboard::from(their_king);
        let atk = &*ATTACKS;

        let moving_piece = if mv.is_promo() {
            mv.promo_piece()
        } else {
            self.moving_piece(mv)
        };

        let mut occ = (self.all_occ ^ from_bb) | to_bb;
        if mv.is_en_passant() {
            let cap_sq = if us == Color::White {
                Square(to.0 - 8)
            } else {
                Square(to.0 + 8)
            };
            occ ^= Bitboard::from(cap_sq);
        }

        let direct = match moving_piece {
            Piece::Pawn => (atk.pawn(us, to) & their_king_bb).any(),
            Piece::Knight => (atk.knight(to) & their_king_bb).any(),
            Piece::Bishop => (atk.bishop(to, occ) & their_king_bb).any(),
            Piece::Rook => (atk.rook(to, occ) & their_king_bb).any(),
            Piece::Queen => (atk.queen(to, occ) & their_king_bb).any(),
            Piece::King => false,
        };
        if direct {
            return true;
        }

        let diagonal_sliders =
            (self.pieces(us, Piece::Bishop) | self.pieces(us, Piece::Queen)) & !from_bb;
        if (atk.bishop(their_king, occ) & diagonal_sliders).any() {
            return true;
        }

        let orthogonal_sliders =
            (self.pieces(us, Piece::Rook) | self.pieces(us, Piece::Queen)) & !from_bb;
        (atk.rook(their_king, occ) & orthogonal_sliders).any()
    }

    fn castling_move(&self, from: Square, to: Square, us: Color, them: Color) -> Option<Move> {
        if self.is_in_check() {
            return None;
        }

        let (king_sq, ks_flag, qs_flag, ks_rook, qs_rook, ks_empty, qs_empty, ks_safe, qs_safe) =
            if us == Color::White {
                (
                    Square::E1,
                    CastlingRights::WHITE_KINGSIDE,
                    CastlingRights::WHITE_QUEENSIDE,
                    Square::H1,
                    Square::A1,
                    Bitboard::from(Square::F1) | Bitboard::from(Square::G1),
                    Bitboard::from(Square::B1)
                        | Bitboard::from(Square::C1)
                        | Bitboard::from(Square::D1),
                    [Square::F1, Square::G1],
                    [Square::D1, Square::C1],
                )
            } else {
                (
                    Square::E8,
                    CastlingRights::BLACK_KINGSIDE,
                    CastlingRights::BLACK_QUEENSIDE,
                    Square::H8,
                    Square::A8,
                    Bitboard::from(Square::F8) | Bitboard::from(Square::G8),
                    Bitboard::from(Square::B8)
                        | Bitboard::from(Square::C8)
                        | Bitboard::from(Square::D8),
                    [Square::F8, Square::G8],
                    [Square::D8, Square::C8],
                )
            };

        if from != king_sq {
            return None;
        }
        if to == ks_safe[1] {
            if self.castling.has(ks_flag)
                && (self.all_occ & ks_empty).is_empty()
                && (self.pieces(us, Piece::Rook) & Bitboard::from(ks_rook)).any()
                && !self.is_attacked(ks_safe[0], them)
                && !self.is_attacked(ks_safe[1], them)
            {
                return Some(Move::new(from, to, CASTLE_KINGSIDE));
            }
        } else if to == qs_safe[1]
            && self.castling.has(qs_flag)
            && (self.all_occ & qs_empty).is_empty()
            && (self.pieces(us, Piece::Rook) & Bitboard::from(qs_rook)).any()
            && !self.is_attacked(qs_safe[0], them)
            && !self.is_attacked(qs_safe[1], them)
        {
            return Some(Move::new(from, to, CASTLE_QUEENSIDE));
        }
        None
    }

    fn king_safe_after(&self, mv: Move) -> bool {
        let us = self.side_to_move;
        let them = !us;
        let from = mv.from_sq();
        let to = mv.to_sq();
        let from_bb = Bitboard::from(from);
        let to_bb = Bitboard::from(to);
        let captured = self.piece_at(to).map(|(_, piece)| (to, piece));

        if self.moving_piece(mv) == Piece::King {
            return mv.is_castling()
                || !self.is_attacked_after_capture_removed(
                    to,
                    them,
                    self.all_occ ^ from_bb,
                    captured,
                );
        }

        let king_sq = self.king_sq(us);
        let (occ_after, captured) = if mv.is_en_passant() {
            let cap_sq = if us == Color::White {
                Square(to.0 - 8)
            } else {
                Square(to.0 + 8)
            };
            (
                (self.all_occ ^ from_bb ^ Bitboard::from(cap_sq)) | to_bb,
                Some((cap_sq, Piece::Pawn)),
            )
        } else {
            ((self.all_occ ^ from_bb) | to_bb, captured)
        };
        !self.is_attacked_after_capture_removed(king_sq, them, occ_after, captured)
    }

    fn is_attacked_after_capture_removed(
        &self,
        sq: Square,
        attacker: Color,
        occ: Bitboard,
        captured: Option<(Square, Piece)>,
    ) -> bool {
        let atk = &*ATTACKS;
        let captured_bb = captured
            .map(|(captured_sq, _)| Bitboard::from(captured_sq))
            .unwrap_or(Bitboard::EMPTY);
        let captured_piece = captured.map(|(_, piece)| piece);
        let pieces = |piece| {
            let bb = self.pieces(attacker, piece);
            if captured_piece == Some(piece) {
                bb & !captured_bb
            } else {
                bb
            }
        };

        if (atk.pawn(!attacker, sq) & pieces(Piece::Pawn)).any() {
            return true;
        }
        if (atk.knight(sq) & pieces(Piece::Knight)).any() {
            return true;
        }
        if (atk.king(sq) & pieces(Piece::King)).any() {
            return true;
        }
        if (atk.bishop(sq, occ) & (pieces(Piece::Bishop) | pieces(Piece::Queen))).any() {
            return true;
        }
        if (atk.rook(sq, occ) & (pieces(Piece::Rook) | pieces(Piece::Queen))).any() {
            return true;
        }
        false
    }

    pub fn can_declare_draw(&self) -> bool {
        self.halfmove_clock >= 100
            || self.has_insufficient_material()
            || self.is_threefold_repetition()
    }

    #[inline(always)]
    pub fn can_declare_draw_in_search(&self) -> bool {
        if self.halfmove_clock >= 100 {
            return true;
        }
        self.has_insufficient_material() || (self.halfmove_clock >= 4 && self.is_repetition(2))
    }

    pub fn has_repeated_position(&self) -> bool {
        self.halfmove_clock >= 4 && self.is_repetition(2)
    }

    pub fn has_non_pawn_material(&self, color: Color) -> bool {
        (self.pieces(color, Piece::Knight)
            | self.pieces(color, Piece::Bishop)
            | self.pieces(color, Piece::Rook)
            | self.pieces(color, Piece::Queen))
        .any()
    }

    #[inline(always)]
    pub fn pawn_key(&self) -> u64 {
        self.pawn_hash
    }

    #[inline(always)]
    pub fn minor_key(&self) -> u64 {
        self.minor_hash
    }

    #[inline(always)]
    pub fn non_pawn_key(&self, color: Color) -> u64 {
        self.non_pawn_hash[color as usize]
    }

    #[inline(always)]
    pub fn attackers_to_color(&self, sq: Square, occ: Bitboard, color: Color) -> Bitboard {
        let atk = &*ATTACKS;
        let diagonal = self.pieces(color, Piece::Bishop) | self.pieces(color, Piece::Queen);
        let orthogonal = self.pieces(color, Piece::Rook) | self.pieces(color, Piece::Queen);

        (atk.pawn(!color, sq) & self.pieces(color, Piece::Pawn)
            | atk.knight(sq) & self.pieces(color, Piece::Knight)
            | atk.king(sq) & self.pieces(color, Piece::King)
            | atk.bishop(sq, occ) & diagonal
            | atk.rook(sq, occ) & orthogonal)
            & occ
    }

    #[inline(always)]
    pub fn see(&self, mv: Move) -> i32 {
        let Some(victim) = self.captured_piece(mv) else {
            return if mv.is_promo() {
                piece_value(mv.promo_piece()) - piece_value(Piece::Pawn)
            } else {
                0
            };
        };

        let target = mv.to_sq();
        let mut occ = self.all_occ;
        let mut side = self.side_to_move;
        let mut gains = [0i32; 32];
        let mut depth = 0usize;

        gains[0] = piece_value(victim);
        if mv.is_promo() {
            gains[0] += piece_value(mv.promo_piece()) - piece_value(Piece::Pawn);
        }

        let from = mv.from_sq();
        let mut attacker_piece = if mv.is_promo() {
            mv.promo_piece()
        } else {
            self.moving_piece(mv)
        };
        occ ^= Bitboard::from(from);
        if mv.is_en_passant() {
            let cap_sq = if side == Color::White {
                Square(target.0 - 8)
            } else {
                Square(target.0 + 8)
            };
            occ ^= Bitboard::from(cap_sq);
        } else {
            occ ^= Bitboard::from(target);
        }
        occ |= Bitboard::from(target);

        loop {
            side = !side;
            let mut attackers = self.legal_see_attackers(target, occ, side);
            if attackers.is_empty() {
                break;
            }

            let (sq, piece) = self.least_valuable_attacker(attackers, side);
            depth += 1;
            gains[depth] = piece_value(attacker_piece) - gains[depth - 1];

            if gains[depth].max(-gains[depth - 1]) < 0 {
                break;
            }

            attacker_piece = piece;
            occ ^= Bitboard::from(sq);
            attackers = self.legal_see_attackers(target, occ, !side);
            if (attackers & self.pieces(!side, Piece::King)).any() {
                break;
            }
        }

        while depth > 0 {
            depth -= 1;
            gains[depth] = -gains[depth + 1].max(-gains[depth]);
        }
        gains[0]
    }

    #[inline(always)]
    pub fn see_ge(&self, mv: Move, threshold: i32) -> bool {
        if !mv.is_capture() {
            let gain = if mv.is_promo() {
                piece_value(mv.promo_piece()) - piece_value(Piece::Pawn)
            } else {
                0
            };
            return gain >= threshold;
        }

        let mut balance = self.captured_piece(mv).map(piece_value).unwrap_or(0);
        if mv.is_promo() {
            balance += piece_value(mv.promo_piece()) - piece_value(Piece::Pawn);
        }
        balance -= threshold;
        if balance < 0 {
            return false;
        }

        let target = mv.to_sq();
        let from = mv.from_sq();
        let mut attacker_piece = if mv.is_promo() {
            mv.promo_piece()
        } else {
            self.moving_piece(mv)
        };

        balance = piece_value(attacker_piece) - balance;
        if balance <= 0 {
            return true;
        }

        let mut occ = self.all_occ ^ Bitboard::from(from);
        if mv.is_en_passant() {
            let cap_sq = if self.side_to_move == Color::White {
                Square(target.0 - 8)
            } else {
                Square(target.0 + 8)
            };
            occ ^= Bitboard::from(cap_sq);
        } else if mv.is_capture() {
            occ ^= Bitboard::from(target);
        }
        occ |= Bitboard::from(target);

        let mut side = self.side_to_move;
        let mut result = true;
        loop {
            side = !side;
            let attackers = self.legal_see_attackers(target, occ, side);
            if attackers.is_empty() {
                break;
            }

            let (sq, piece) = self.least_valuable_attacker(attackers, side);
            attacker_piece = piece;
            occ ^= Bitboard::from(sq);

            let next_attackers = self.legal_see_attackers(target, occ, !side);
            if (next_attackers & self.pieces(!side, Piece::King)).any() {
                break;
            }

            balance = piece_value(attacker_piece) - balance;
            result = !result;
            if result == (balance >= 0) {
                break;
            }
        }

        result
    }

    pub fn game_result(&self) -> Option<GameResult> {
        if self.can_declare_draw() {
            return Some(GameResult::Draw);
        }

        if !generate_legal_moves(self).is_empty() {
            return None;
        }

        if self.is_in_check() {
            match self.side_to_move {
                Color::White => Some(GameResult::BlackCheckmates),
                Color::Black => Some(GameResult::WhiteCheckmates),
            }
        } else {
            Some(GameResult::Stalemate)
        }
    }

    // -----------------------------------------------------------------------
    // Check / attack queries
    // -----------------------------------------------------------------------

    /// Is the given square attacked by any piece of `attacker_color`?
    #[inline(always)]
    pub fn is_attacked(&self, sq: Square, attacker: Color) -> bool {
        let occ = self.all_occ;
        let atk = &*ATTACKS;

        // Pawn attacks
        if (atk.pawn(!attacker, sq) & self.pieces(attacker, Piece::Pawn)).any() {
            return true;
        }
        // Knight
        if (atk.knight(sq) & self.pieces(attacker, Piece::Knight)).any() {
            return true;
        }
        // King
        if (atk.king(sq) & self.pieces(attacker, Piece::King)).any() {
            return true;
        }
        // Bishop / Queen (diagonal)
        if (atk.bishop(sq, occ)
            & (self.pieces(attacker, Piece::Bishop) | self.pieces(attacker, Piece::Queen)))
        .any()
        {
            return true;
        }
        // Rook / Queen (orthogonal)
        if (atk.rook(sq, occ)
            & (self.pieces(attacker, Piece::Rook) | self.pieces(attacker, Piece::Queen)))
        .any()
        {
            return true;
        }
        false
    }

    /// Is the side-to-move's king currently in check?
    #[inline(always)]
    pub fn is_in_check(&self) -> bool {
        self.checkers.any()
    }

    #[inline(always)]
    pub fn checkers(&self) -> Bitboard {
        self.checkers
    }

    /// Bitboard of all pieces that attack the given square (any color).
    #[inline(always)]
    pub fn attackers_to(&self, sq: Square, occ: Bitboard) -> Bitboard {
        let atk = &*ATTACKS;
        atk.pawn(Color::Black, sq) & self.pieces(Color::White, Piece::Pawn)
            | atk.pawn(Color::White, sq) & self.pieces(Color::Black, Piece::Pawn)
            | atk.knight(sq)
                & (self.pieces(Color::White, Piece::Knight)
                    | self.pieces(Color::Black, Piece::Knight))
            | atk.king(sq)
                & (self.pieces(Color::White, Piece::King) | self.pieces(Color::Black, Piece::King))
            | atk.bishop(sq, occ)
                & (self.pieces(Color::White, Piece::Bishop)
                    | self.pieces(Color::Black, Piece::Bishop)
                    | self.pieces(Color::White, Piece::Queen)
                    | self.pieces(Color::Black, Piece::Queen))
            | atk.rook(sq, occ)
                & (self.pieces(Color::White, Piece::Rook)
                    | self.pieces(Color::Black, Piece::Rook)
                    | self.pieces(Color::White, Piece::Queen)
                    | self.pieces(Color::Black, Piece::Queen))
    }

    // -----------------------------------------------------------------------
    // Make / Unmake
    // -----------------------------------------------------------------------

    /// Apply a move in-place.  The move must be legal.
    #[inline(always)]
    pub fn make_move(&mut self, mv: Move) {
        let from = mv.from_sq();
        let to = mv.to_sq();
        let flags = mv.flags();
        let us = self.side_to_move;
        let them = !us;

        let zob = &ZOBRIST;

        let old_castling = self.castling;
        let old_ep_sq = self.ep_sq;
        let old_halfmove_clock = self.halfmove_clock;
        let old_fullmove = self.fullmove;
        let old_hash = self.hash;
        let old_checkers = self.checkers;
        let old_piece_threats = self.piece_threats;
        let old_all_threats = self.all_threats;
        let mut captured = 255;

        // Halfmove clock: reset on pawn move or capture; increment otherwise.
        // We set it properly below after determining if it's a pawn move.

        // Remove old EP contribution from hash
        if self.ep_sq != 255 {
            self.hash ^= zob.ep(Square(self.ep_sq).file());
        }
        self.ep_sq = 255;

        debug_assert!(self.mailbox[from.index()] < 12);
        let moving_piece = self.piece_type_at_unchecked(from);

        // Remove moving piece from origin
        self.remove_piece(us, moving_piece, from);
        self.hash ^= zob.piece(us, moving_piece, from);

        // Handle en passant capture
        if flags == EN_PASSANT {
            let ep_cap_sq = if us == Color::White {
                Square(to.0 - 8)
            } else {
                Square(to.0 + 8)
            };
            captured = encode_piece(them, Piece::Pawn);
            self.remove_piece(them, Piece::Pawn, ep_cap_sq);
            self.hash ^= zob.piece(them, Piece::Pawn, ep_cap_sq);
            self.halfmove_clock = 0;
        } else if flags == CAPTURE || flags >= PROMO_CAPTURE_KNIGHT {
            // Regular capture (including promo-captures)
            debug_assert!(self.mailbox[to.index()] < 12);
            let captured_piece = self.piece_type_at_unchecked(to);
            captured = encode_piece(them, captured_piece);
            self.remove_piece(them, captured_piece, to);
            self.hash ^= zob.piece(them, captured_piece, to);
            self.halfmove_clock = 0;
        } else if moving_piece == Piece::Pawn {
            self.halfmove_clock = 0;
        } else {
            self.halfmove_clock = self.halfmove_clock.saturating_add(1);
        }

        // Place moving piece on destination (or promotion piece)
        if flags >= PROMO_KNIGHT {
            let promo = mv.promo_piece();
            self.add_piece(us, promo, to);
            self.hash ^= zob.piece(us, promo, to);
        } else {
            self.add_piece(us, moving_piece, to);
            self.hash ^= zob.piece(us, moving_piece, to);
        }

        // Castling: move the rook as well
        match flags {
            CASTLE_KINGSIDE => {
                let (rook_from, rook_to) = if us == Color::White {
                    (Square::H1, Square::F1)
                } else {
                    (Square::H8, Square::F8)
                };
                self.remove_piece(us, Piece::Rook, rook_from);
                self.hash ^= zob.piece(us, Piece::Rook, rook_from);
                self.add_piece(us, Piece::Rook, rook_to);
                self.hash ^= zob.piece(us, Piece::Rook, rook_to);
            }
            CASTLE_QUEENSIDE => {
                let (rook_from, rook_to) = if us == Color::White {
                    (Square::A1, Square::D1)
                } else {
                    (Square::A8, Square::D8)
                };
                self.remove_piece(us, Piece::Rook, rook_from);
                self.hash ^= zob.piece(us, Piece::Rook, rook_from);
                self.add_piece(us, Piece::Rook, rook_to);
                self.hash ^= zob.piece(us, Piece::Rook, rook_to);
            }
            DOUBLE_PUSH => {
                // Set en passant square (one step behind the destination)
                let ep = if us == Color::White {
                    Square(to.0 - 8)
                } else {
                    Square(to.0 + 8)
                };
                if self.legal_ep_capture_exists(them, ep).unwrap_or(false) {
                    self.ep_sq = ep.0;
                    self.hash ^= zob.ep(ep.file());
                }
            }
            _ => {}
        }

        // Update castling rights
        let new_castling = self.castling.update(from, to);
        if new_castling != self.castling {
            self.hash ^= zob.castling(self.castling) ^ zob.castling(new_castling);
            self.castling = new_castling;
        }

        // Flip side to move
        self.hash ^= zob.side();
        self.side_to_move = them;

        // Fullmove counter
        if us == Color::Black {
            self.fullmove += 1;
        }
        self.history.push(UnmakeInfo {
            captured,
            castling: old_castling,
            ep_sq: old_ep_sq,
            halfmove_clock: old_halfmove_clock,
            fullmove: old_fullmove,
            hash: old_hash,
            checkers: old_checkers,
            piece_threats: old_piece_threats,
            all_threats: old_all_threats,
        });
        self.checkers = self.calculate_checkers();
        self.update_threats();
    }

    pub fn make_null_move(&mut self) {
        debug_assert!(!self.is_in_check(), "null move while in check");
        let old_castling = self.castling;
        let old_ep_sq = self.ep_sq;
        let old_halfmove_clock = self.halfmove_clock;
        let old_fullmove = self.fullmove;
        let old_hash = self.hash;
        let old_checkers = self.checkers;
        let old_piece_threats = self.piece_threats;
        let old_all_threats = self.all_threats;

        if self.ep_sq != 255 {
            self.hash ^= ZOBRIST.ep(Square(self.ep_sq).file());
            self.ep_sq = 255;
        }
        if self.side_to_move == Color::Black {
            self.fullmove += 1;
        }
        self.halfmove_clock = self.halfmove_clock.saturating_add(1);
        self.side_to_move = !self.side_to_move;
        self.hash ^= ZOBRIST.side();
        self.history.push(UnmakeInfo {
            captured: NO_PIECE,
            castling: old_castling,
            ep_sq: old_ep_sq,
            halfmove_clock: old_halfmove_clock,
            fullmove: old_fullmove,
            hash: old_hash,
            checkers: old_checkers,
            piece_threats: old_piece_threats,
            all_threats: old_all_threats,
        });
        self.checkers = Bitboard::EMPTY;
        self.update_threats();
    }

    pub fn unmake_null_move(&mut self) {
        let info = self
            .history
            .pop()
            .expect("unmake_null_move with empty history");
        debug_assert_eq!(info.captured, NO_PIECE);
        self.side_to_move = !self.side_to_move;
        self.castling = info.castling;
        self.ep_sq = info.ep_sq;
        self.halfmove_clock = info.halfmove_clock;
        self.fullmove = info.fullmove;
        self.hash = info.hash;
        self.checkers = info.checkers;
        self.piece_threats = info.piece_threats;
        self.all_threats = info.all_threats;
    }

    /// Undo the last move.
    #[inline(always)]
    pub fn unmake_move(&mut self, mv: Move) {
        let info = self.history.pop().expect("unmake_move with empty history");

        let from = mv.from_sq();
        let to = mv.to_sq();
        let flags = mv.flags();

        // Restore side to move (it was flipped by make_move)
        self.side_to_move = !self.side_to_move;
        let us = self.side_to_move;
        let _them = !us;

        // Restore state fields
        self.castling = info.castling;
        self.ep_sq = info.ep_sq;
        self.halfmove_clock = info.halfmove_clock;
        self.fullmove = info.fullmove;
        self.hash = info.hash;
        self.checkers = info.checkers;
        self.piece_threats = info.piece_threats;
        self.all_threats = info.all_threats;

        // Move the piece back from `to` to `from`
        let moved_piece = if flags >= PROMO_KNIGHT {
            // Promotion: remove the promo piece, restore a pawn
            let promo = mv.promo_piece();
            self.remove_piece(us, promo, to);
            Piece::Pawn
        } else {
            debug_assert!(self.mailbox[to.index()] < 12);
            let p = self.piece_type_at_unchecked(to);
            self.remove_piece(us, p, to);
            p
        };

        self.add_piece(us, moved_piece, from);

        // Restore captured piece
        if info.captured != 255 {
            let cap_color = if info.captured < 6 {
                Color::White
            } else {
                Color::Black
            };
            let cap_piece = PIECE_FROM_ENCODED[info.captured as usize];
            let cap_sq = if flags == EN_PASSANT {
                if us == Color::White {
                    Square(to.0 - 8)
                } else {
                    Square(to.0 + 8)
                }
            } else {
                to
            };
            self.add_piece(cap_color, cap_piece, cap_sq);
        }

        // Undo castling rook move
        match flags {
            CASTLE_KINGSIDE => {
                let (rook_from, rook_to) = if us == Color::White {
                    (Square::H1, Square::F1)
                } else {
                    (Square::H8, Square::F8)
                };
                self.remove_piece(us, Piece::Rook, rook_to);
                self.add_piece(us, Piece::Rook, rook_from);
            }
            CASTLE_QUEENSIDE => {
                let (rook_from, rook_to) = if us == Color::White {
                    (Square::A1, Square::D1)
                } else {
                    (Square::A8, Square::D8)
                };
                self.remove_piece(us, Piece::Rook, rook_to);
                self.add_piece(us, Piece::Rook, rook_from);
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    #[inline(always)]
    fn add_piece(&mut self, color: Color, piece: Piece, sq: Square) {
        let bb = Bitboard::from(sq);
        self.mailbox[sq.index()] = encode_piece(color, piece);
        self.pieces[color as usize * 6 + piece as usize] |= bb;
        self.occupancy[color as usize] |= bb;
        self.all_occ |= bb;
        let piece_key = ZOBRIST.piece(color, piece, sq);
        match piece {
            Piece::Pawn => self.pawn_hash ^= piece_key,
            Piece::Knight | Piece::Bishop => {
                self.minor_hash ^= piece_key;
                self.non_pawn_hash[color as usize] ^= piece_key;
            }
            Piece::Rook | Piece::Queen => self.non_pawn_hash[color as usize] ^= piece_key,
            Piece::King => {}
        }
    }

    #[inline(always)]
    fn remove_piece(&mut self, color: Color, piece: Piece, sq: Square) {
        let bb = Bitboard::from(sq);
        self.mailbox[sq.index()] = NO_PIECE;
        self.pieces[color as usize * 6 + piece as usize] ^= bb;
        self.occupancy[color as usize] ^= bb;
        self.all_occ ^= bb;
        let piece_key = ZOBRIST.piece(color, piece, sq);
        match piece {
            Piece::Pawn => self.pawn_hash ^= piece_key,
            Piece::Knight | Piece::Bishop => {
                self.minor_hash ^= piece_key;
                self.non_pawn_hash[color as usize] ^= piece_key;
            }
            Piece::Rook | Piece::Queen => self.non_pawn_hash[color as usize] ^= piece_key,
            Piece::King => {}
        }
    }

    #[inline(always)]
    fn piece_type_at_unchecked(&self, sq: Square) -> Piece {
        debug_assert!(self.mailbox[sq.index()] < 12);
        unsafe { *PIECE_FROM_ENCODED.get_unchecked(self.mailbox[sq.index()] as usize) }
    }

    #[inline(always)]
    fn least_valuable_attacker(&self, attackers: Bitboard, color: Color) -> (Square, Piece) {
        let pawns = attackers & self.pieces(color, Piece::Pawn);
        if pawns.any() {
            return (pawns.lsb(), Piece::Pawn);
        }
        let knights = attackers & self.pieces(color, Piece::Knight);
        if knights.any() {
            return (knights.lsb(), Piece::Knight);
        }
        let bishops = attackers & self.pieces(color, Piece::Bishop);
        if bishops.any() {
            return (bishops.lsb(), Piece::Bishop);
        }
        let rooks = attackers & self.pieces(color, Piece::Rook);
        if rooks.any() {
            return (rooks.lsb(), Piece::Rook);
        }
        let queens = attackers & self.pieces(color, Piece::Queen);
        if queens.any() {
            return (queens.lsb(), Piece::Queen);
        }
        let kings = attackers & self.pieces(color, Piece::King);
        if kings.any() {
            return (kings.lsb(), Piece::King);
        }
        unreachable!("least_valuable_attacker called with no attackers")
    }

    fn update_threats(&mut self) {
        self.piece_threats = self.calculate_piece_threats(!self.side_to_move);
        self.all_threats = self
            .piece_threats
            .iter()
            .copied()
            .fold(Bitboard::EMPTY, |acc, threats| acc | threats);
    }

    fn calculate_piece_threats(&self, attacker: Color) -> [Bitboard; 6] {
        let atk = &*ATTACKS;
        let occ = self.all_occ;
        let mut threats = [Bitboard::EMPTY; 6];

        let pawns = self.pieces(attacker, Piece::Pawn);
        threats[Piece::Pawn as usize] = if attacker == Color::White {
            pawns.north_east() | pawns.north_west()
        } else {
            pawns.south_east() | pawns.south_west()
        };

        let mut knights = self.pieces(attacker, Piece::Knight);
        while knights.any() {
            threats[Piece::Knight as usize] |= atk.knight(knights.pop_lsb());
        }

        let mut bishops = self.pieces(attacker, Piece::Bishop);
        while bishops.any() {
            threats[Piece::Bishop as usize] |= atk.bishop(bishops.pop_lsb(), occ);
        }

        let mut rooks = self.pieces(attacker, Piece::Rook);
        while rooks.any() {
            threats[Piece::Rook as usize] |= atk.rook(rooks.pop_lsb(), occ);
        }

        let mut queens = self.pieces(attacker, Piece::Queen);
        while queens.any() {
            let sq = queens.pop_lsb();
            threats[Piece::Queen as usize] |= atk.bishop(sq, occ) | atk.rook(sq, occ);
        }

        threats[Piece::King as usize] = atk.king(self.king_sq(attacker));
        threats
    }

    #[inline(always)]
    fn legal_see_attackers(&self, target: Square, occ: Bitboard, color: Color) -> Bitboard {
        let mut attackers = self.attackers_to_color(target, occ, color);
        let mut legal = Bitboard::EMPTY;
        while attackers.any() {
            let from = attackers.pop_lsb();
            if self.see_attacker_is_legal(from, target, occ, color) {
                legal |= Bitboard::from(from);
            }
        }
        legal
    }

    #[inline(always)]
    fn see_attacker_is_legal(
        &self,
        from: Square,
        target: Square,
        occ: Bitboard,
        color: Color,
    ) -> bool {
        let from_bb = Bitboard::from(from);
        let target_bb = Bitboard::from(target);
        let occ_after = (occ ^ from_bb) | target_bb;
        let king = self.king_sq(color);
        let them = !color;
        let atk = &*ATTACKS;

        if (from_bb & self.pieces(color, Piece::King)).any() {
            return self.attackers_to_color(target, occ_after, them).is_empty();
        }

        let diagonal = (self.pieces(them, Piece::Bishop) | self.pieces(them, Piece::Queen))
            & occ_after
            & atk.bishop(king, occ_after);
        let orthogonal = (self.pieces(them, Piece::Rook) | self.pieces(them, Piece::Queen))
            & occ_after
            & atk.rook(king, occ_after);
        (diagonal | orthogonal).is_empty()
    }

    #[inline(always)]
    fn calculate_checkers(&self) -> Bitboard {
        self.attackers_to(self.king_sq(self.side_to_move), self.all_occ)
            & self.color_occ(!self.side_to_move)
    }

    fn validate_position(&self) -> Result<(), String> {
        let white_king = self.pieces(Color::White, Piece::King);
        let black_king = self.pieces(Color::Black, Piece::King);
        if white_king.count() != 1 || black_king.count() != 1 {
            return Err("FEN must contain exactly one king for each side".to_string());
        }

        let white_king_sq = white_king.lsb();
        let black_king_sq = black_king.lsb();
        if white_king_sq.chebyshev_distance(black_king_sq) <= 1 {
            return Err("kings may not be adjacent".to_string());
        }

        let just_moved = !self.side_to_move;
        if self.is_attacked(self.king_sq(just_moved), self.side_to_move) {
            return Err("side not to move may not be in check".to_string());
        }

        Ok(())
    }

    fn set_legal_ep_square(&mut self, ep_sq: Square) -> Result<(), String> {
        let capturer = self.side_to_move;
        let captured = !capturer;
        let expected_rank = if capturer == Color::White { 5 } else { 2 };
        if ep_sq.rank() as u8 != expected_rank {
            return Err(format!("invalid en passant rank: {ep_sq}"));
        }
        if self.piece_at(ep_sq).is_some() {
            return Err(format!("en passant target square is occupied: {ep_sq}"));
        }

        let cap_sq = ep_capture_square(capturer, ep_sq)
            .ok_or_else(|| format!("invalid en passant square: {ep_sq}"))?;
        if self.piece_at(cap_sq) != Some((captured, Piece::Pawn)) {
            return Err(format!("missing en passant capturable pawn at {cap_sq}"));
        }

        let origin_sq = ep_origin_square(capturer, ep_sq)
            .ok_or_else(|| format!("invalid en passant square: {ep_sq}"))?;
        if self.piece_at(origin_sq).is_some() {
            return Err(format!("en passant origin square is occupied: {origin_sq}"));
        }

        if self.legal_ep_capture_exists(capturer, ep_sq)? {
            self.ep_sq = ep_sq.0;
            self.hash ^= ZOBRIST.ep(ep_sq.file());
        }
        Ok(())
    }

    fn legal_ep_capture_exists(&self, capturer: Color, ep_sq: Square) -> Result<bool, String> {
        let Some(cap_sq) = ep_capture_square(capturer, ep_sq) else {
            return Ok(false);
        };
        let atk = &*ATTACKS;
        let mut attackers = atk.pawn(!capturer, ep_sq) & self.pieces(capturer, Piece::Pawn);
        while attackers.any() {
            let from = attackers.pop_lsb();
            if self.ep_capture_is_legal(capturer, from, ep_sq, cap_sq) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn ep_capture_is_legal(
        &self,
        capturer: Color,
        from: Square,
        ep_sq: Square,
        cap_sq: Square,
    ) -> bool {
        let them = !capturer;
        let king_sq = self.king_sq(capturer);
        let occ_after =
            (self.all_occ ^ Bitboard::from(from) ^ Bitboard::from(cap_sq)) | Bitboard::from(ep_sq);
        let atk = &*ATTACKS;
        let exposed_rook = (self.pieces(them, Piece::Rook) | self.pieces(them, Piece::Queen))
            & atk.rook(king_sq, occ_after);
        let exposed_diag = (self.pieces(them, Piece::Bishop) | self.pieces(them, Piece::Queen))
            & atk.bishop(king_sq, occ_after);

        exposed_rook.is_empty() && exposed_diag.is_empty()
    }

    fn has_insufficient_material(&self) -> bool {
        if (self.pieces(Color::White, Piece::Pawn)
            | self.pieces(Color::Black, Piece::Pawn)
            | self.pieces(Color::White, Piece::Rook)
            | self.pieces(Color::Black, Piece::Rook)
            | self.pieces(Color::White, Piece::Queen)
            | self.pieces(Color::Black, Piece::Queen))
        .any()
        {
            return false;
        }

        let knights =
            self.pieces(Color::White, Piece::Knight) | self.pieces(Color::Black, Piece::Knight);
        let bishops =
            self.pieces(Color::White, Piece::Bishop) | self.pieces(Color::Black, Piece::Bishop);
        let minors = knights | bishops;
        if minors.count() <= 1 {
            return true;
        }
        if knights.any() {
            return false;
        }

        let mut bishop_squares = bishops;
        let mut color_complex: Option<u8> = None;
        while bishop_squares.any() {
            let sq = bishop_squares.pop_lsb();
            let complex = (sq.file() as u8 + sq.rank() as u8) & 1;
            if color_complex.is_some_and(|known| known != complex) {
                return false;
            }
            color_complex = Some(complex);
        }
        true
    }

    fn is_threefold_repetition(&self) -> bool {
        self.is_repetition(3)
    }

    fn is_repetition(&self, needed_count: usize) -> bool {
        let mut count = 1usize;
        let max_plies = self.halfmove_clock as usize;
        let mut plies_back = 2usize;

        while plies_back <= max_plies && plies_back <= self.history.len() {
            if self.history[self.history.len() - plies_back].hash == self.hash {
                count += 1;
                if count >= needed_count {
                    return true;
                }
            }
            plies_back += 2;
        }

        false
    }
}

impl Default for Board {
    fn default() -> Self {
        Board::starting_position()
    }
}

// -----------------------------------------------------------------------
// FEN helper
// -----------------------------------------------------------------------

fn validate_castling_rights(board: &Board, rights: CastlingRights) -> Result<(), String> {
    let required = [
        (
            CastlingRights::WHITE_KINGSIDE,
            Square::E1,
            Square::H1,
            Color::White,
        ),
        (
            CastlingRights::WHITE_QUEENSIDE,
            Square::E1,
            Square::A1,
            Color::White,
        ),
        (
            CastlingRights::BLACK_KINGSIDE,
            Square::E8,
            Square::H8,
            Color::Black,
        ),
        (
            CastlingRights::BLACK_QUEENSIDE,
            Square::E8,
            Square::A8,
            Color::Black,
        ),
    ];

    for (right, king_sq, rook_sq, color) in required {
        if rights.has(right) {
            if board.piece_at(king_sq) != Some((color, Piece::King))
                || board.piece_at(rook_sq) != Some((color, Piece::Rook))
            {
                return Err(format!(
                    "castling right {} does not match king/rook placement",
                    right.as_str()
                ));
            }
        }
    }

    Ok(())
}

fn ep_capture_square(capturer: Color, ep_sq: Square) -> Option<Square> {
    match capturer {
        Color::White => ep_sq.0.checked_sub(8).map(Square),
        Color::Black => ep_sq.0.checked_add(8).filter(|sq| *sq < 64).map(Square),
    }
}

fn ep_origin_square(capturer: Color, ep_sq: Square) -> Option<Square> {
    match capturer {
        Color::White => ep_sq.0.checked_add(8).filter(|sq| *sq < 64).map(Square),
        Color::Black => ep_sq.0.checked_sub(8).map(Square),
    }
}

fn fen_char_to_piece(c: char) -> Result<(Color, Piece), String> {
    match c {
        'P' => Ok((Color::White, Piece::Pawn)),
        'N' => Ok((Color::White, Piece::Knight)),
        'B' => Ok((Color::White, Piece::Bishop)),
        'R' => Ok((Color::White, Piece::Rook)),
        'Q' => Ok((Color::White, Piece::Queen)),
        'K' => Ok((Color::White, Piece::King)),
        'p' => Ok((Color::Black, Piece::Pawn)),
        'n' => Ok((Color::Black, Piece::Knight)),
        'b' => Ok((Color::Black, Piece::Bishop)),
        'r' => Ok((Color::Black, Piece::Rook)),
        'q' => Ok((Color::Black, Piece::Queen)),
        'k' => Ok((Color::Black, Piece::King)),
        c => Err(format!("invalid FEN piece char: {c}")),
    }
}

#[inline(always)]
fn encode_piece(color: Color, piece: Piece) -> u8 {
    color as u8 * 6 + piece as u8
}

#[inline(always)]
fn capture_or_quiet(target: Option<Color>, them: Color) -> u16 {
    if target == Some(them) { CAPTURE } else { QUIET }
}

#[inline(always)]
fn pawn_move_with_promotion(
    from: Square,
    to: Square,
    is_capture: bool,
    promotion: Option<Piece>,
) -> Option<Move> {
    let flag = match promotion {
        Some(piece) => promotion_flag(piece, is_capture)?,
        None if is_capture => CAPTURE,
        None => QUIET,
    };
    Some(Move::new(from, to, flag))
}

#[inline(always)]
fn promotion_flag(piece: Piece, is_capture: bool) -> Option<u16> {
    match (piece, is_capture) {
        (Piece::Knight, false) => Some(PROMO_KNIGHT),
        (Piece::Bishop, false) => Some(PROMO_BISHOP),
        (Piece::Rook, false) => Some(PROMO_ROOK),
        (Piece::Queen, false) => Some(PROMO_QUEEN),
        (Piece::Knight, true) => Some(PROMO_CAPTURE_KNIGHT),
        (Piece::Bishop, true) => Some(PROMO_CAPTURE_BISHOP),
        (Piece::Rook, true) => Some(PROMO_CAPTURE_ROOK),
        (Piece::Queen, true) => Some(PROMO_CAPTURE_QUEEN),
        _ => None,
    }
}

#[inline(always)]
fn decode_piece(encoded: u8) -> Option<(Color, Piece)> {
    if encoded >= 12 {
        return None;
    }

    let color = if encoded < 6 {
        Color::White
    } else {
        Color::Black
    };
    let piece = PIECE_FROM_ENCODED[encoded as usize];
    Some((color, piece))
}

#[inline(always)]
fn decode_piece_type(encoded: u8) -> Option<Piece> {
    if encoded < 12 {
        Some(PIECE_FROM_ENCODED[encoded as usize])
    } else {
        None
    }
}

#[inline(always)]
fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Knight => 320,
        Piece::Bishop => 330,
        Piece::Rook => 500,
        Piece::Queen => 900,
        Piece::King => 20_000,
    }
}

// -----------------------------------------------------------------------
// Display
// -----------------------------------------------------------------------

impl fmt::Display for Board {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  +-----------------+")?;
        for rank in (0..8).rev() {
            write!(f, "{} | ", rank + 1)?;
            for file in 0..8u8 {
                let sq = Square(rank * 8 + file);
                if let Some((color, piece)) = self.piece_at(sq) {
                    let c = match piece {
                        Piece::Pawn => 'p',
                        Piece::Knight => 'n',
                        Piece::Bishop => 'b',
                        Piece::Rook => 'r',
                        Piece::Queen => 'q',
                        Piece::King => 'k',
                    };
                    let c = if color == Color::White {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    write!(f, "{c} ")?;
                } else {
                    write!(f, ". ")?;
                }
            }
            writeln!(f, "|")?;
        }
        writeln!(f, "  +-----------------+")?;
        writeln!(f, "    a b c d e f g h")?;
        writeln!(f, "  Side: {:?}", self.side_to_move)?;
        writeln!(f, "  Castling: {}", self.castling.as_str())?;
        if self.ep_sq != 255 {
            writeln!(f, "  EP: {}", Square(self.ep_sq))?;
        }
        writeln!(f, "  Hash: 0x{:016X}", self.hash)?;
        Ok(())
    }
}
