use std::{mem::MaybeUninit, slice};

use crate::board::{Move, Piece};

pub const HISTORY_MAX: i32 = 16_384;
pub const CAP_HISTORY_MAX: i32 = 16_384;
pub const CORR_SIZE: usize = 16_384;
pub const CONT_SIZE: usize = 6 * 64 * 6 * 64;
pub const LOW_PLY_HISTORY_SIZE: usize = 8;
pub const PAWN_HISTORY_SIZE: usize = 4_096;
pub const PIECE_TO_SIZE: usize = 6 * 64;
pub(crate) const CHECK_UNKNOWN: i8 = -1;
pub(crate) const CHECK_FALSE: i8 = 0;
pub(crate) const CHECK_TRUE: i8 = 1;

#[derive(Copy, Clone)]
pub(crate) struct ScoredMove {
    pub mv: Move,
    pub score: i32,
    pub see: i16,
    pub quiet_history: i32,
    pub gives_check: i8,
}

impl Default for ScoredMove {
    fn default() -> Self {
        Self {
            mv: Move::NULL,
            score: 0,
            see: 0,
            quiet_history: 0,
            gives_check: CHECK_UNKNOWN,
        }
    }
}

pub(crate) struct ScoredMoveList {
    moves: [MaybeUninit<ScoredMove>; 256],
    len: usize,
}

impl ScoredMoveList {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            moves: uninit_array(),
            len: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, mv: Move, score: i32, see: i32) {
        self.push_with_history(mv, score, see, 0);
    }

    #[inline(always)]
    pub fn push_with_history(&mut self, mv: Move, score: i32, see: i32, quiet_history: i32) {
        self.push_with_history_and_check(mv, score, see, quiet_history, CHECK_UNKNOWN);
    }

    #[inline(always)]
    pub fn push_with_history_and_check(
        &mut self,
        mv: Move,
        score: i32,
        see: i32,
        quiet_history: i32,
        gives_check: i8,
    ) {
        debug_assert!(self.len < self.moves.len());
        self.moves[self.len].write(ScoredMove {
            mv,
            score,
            see: see.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            quiet_history,
            gives_check,
        });
        self.len += 1;
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline(always)]
    pub fn as_mut_slice(&mut self) -> &mut [ScoredMove] {
        // Only the initialized prefix is exposed.
        unsafe { slice::from_raw_parts_mut(self.moves.as_mut_ptr().cast::<ScoredMove>(), self.len) }
    }
}

#[derive(Copy, Clone)]
pub(crate) struct BadCapture {
    pub attacker: Piece,
    pub to: usize,
    pub captured: Option<Piece>,
}

pub(crate) struct BadCaptureList {
    items: [MaybeUninit<BadCapture>; 256],
    len: usize,
}

impl BadCaptureList {
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            items: uninit_array(),
            len: 0,
        }
    }

    #[inline(always)]
    pub fn push(&mut self, attacker: Piece, to: usize, captured: Option<Piece>) {
        debug_assert!(self.len < self.items.len());
        self.items[self.len].write(BadCapture {
            attacker,
            to,
            captured,
        });
        self.len += 1;
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &[BadCapture] {
        // Only the initialized prefix is exposed.
        unsafe { slice::from_raw_parts(self.items.as_ptr().cast::<BadCapture>(), self.len) }
    }
}

pub(crate) fn pick_next(moves: &mut [ScoredMove], index: usize) -> ScoredMove {
    let mut best = index;
    for current in index + 1..moves.len() {
        if moves[current].score > moves[best].score {
            best = current;
        }
    }
    moves.swap(index, best);
    moves[index]
}

pub(crate) fn diversify_root_scores(moves: &mut [ScoredMove], offset: usize) {
    moves.sort_unstable_by(|left, right| right.score.cmp(&left.score));
    if offset < moves.len() {
        moves[offset].score = moves[0].score.saturating_add(1_000_000);
    }
}

pub(crate) fn update_hist_entry(entry: &mut i16, bonus: i32, max_value: i32) {
    let current = *entry as i32;
    let updated = current + bonus - current * bonus.abs() / max_value;
    *entry = updated.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
}

pub(crate) fn history_bonus(depth: i32) -> i32 {
    (depth * depth + 2 * depth).min(1_200)
}

pub(crate) fn cont_index(prev_piece: usize, prev_to: usize, piece: usize, to: usize) -> usize {
    (((prev_piece * 64 + prev_to) * 6 + piece) * 64 + to).min(CONT_SIZE - 1)
}

pub(crate) fn piece_to_index(piece: usize, to: usize) -> usize {
    (piece * 64 + to).min(PIECE_TO_SIZE - 1)
}

pub(crate) fn pawn_history_index(pawn_key: u64, piece: usize, to: usize) -> usize {
    let slot = pawn_key as usize & (PAWN_HISTORY_SIZE - 1);
    slot * PIECE_TO_SIZE + piece_to_index(piece, to)
}

#[inline(always)]
fn uninit_array<T, const N: usize>() -> [MaybeUninit<T>; N] {
    // An array of `MaybeUninit<T>` is valid without initializing its elements.
    unsafe { MaybeUninit::<[MaybeUninit<T>; N]>::uninit().assume_init() }
}
