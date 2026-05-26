use crate::board::Color;
use crate::search_options::{EngineOptions, SearchLimits};

#[derive(Copy, Clone)]
pub(crate) struct RuntimeLimits {
    pub depth: usize,
    pub nodes: u64,
    pub soft_ms: f64,
    pub hard_ms: f64,
}

pub(crate) fn compute_runtime_limits(
    options: SearchLimits,
    engine_options: &EngineOptions,
    side_to_move: Color,
    max_depth: usize,
) -> RuntimeLimits {
    let depth = if options.depth.is_finite() {
        options.depth.max(1.0) as usize
    } else {
        max_depth
    };
    let mut soft_ms = f64::INFINITY;
    let mut hard_ms = f64::INFINITY;

    if options.move_time > 0 {
        let available = (options.move_time as f64 - engine_options.move_overhead).max(1.0);
        soft_ms = available;
        hard_ms = available;
    } else {
        let (time, increment) = match side_to_move {
            Color::White => (options.white_time, options.white_increment),
            Color::Black => (options.black_time, options.black_increment),
        };
        if time > 0 {
            let remaining = (time as f64 - engine_options.move_overhead).max(1.0);
            let moves_to_go = if options.movestogo > 0 {
                options.movestogo as f64
            } else {
                35.0
            };
            soft_ms = (remaining / moves_to_go + increment as f64 * 0.70).max(1.0);
            hard_ms = (soft_ms * 3.0).min(remaining * 0.6).max(soft_ms);
        }
    }

    RuntimeLimits {
        depth,
        nodes: options.nodes,
        soft_ms,
        hard_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < f64::EPSILON,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn movetime_uses_exact_budget_after_overhead() {
        let mut engine = EngineOptions::default();
        engine.move_overhead = 25.0;
        let options = SearchLimits {
            move_time: 250,
            white_time: 1,
            black_time: 1,
            white_increment: 1_000,
            black_increment: 1_000,
            ..SearchLimits::default()
        };
        let limits = compute_runtime_limits(options, &engine, Color::White, 64);

        assert_close(limits.soft_ms, 225.0);
        assert_close(limits.hard_ms, 225.0);
    }

    #[test]
    fn movetime_never_allocates_less_than_one_millisecond() {
        let mut engine = EngineOptions::default();
        engine.move_overhead = 10.0;
        let options = SearchLimits {
            move_time: 5,
            ..SearchLimits::default()
        };
        let limits = compute_runtime_limits(options, &engine, Color::Black, 64);

        assert_close(limits.soft_ms, 1.0);
        assert_close(limits.hard_ms, 1.0);
    }

    #[test]
    fn sudden_death_time_control_keeps_a_hard_reserve() {
        let options = SearchLimits {
            white_time: 1000,
            white_increment: 100,
            ..SearchLimits::default()
        };
        let limits = compute_runtime_limits(options, &EngineOptions::default(), Color::White, 64);

        assert!(limits.soft_ms < 100.0);
        assert!(limits.hard_ms < 300.0);
    }

    #[test]
    fn clock_selection_uses_side_to_move() {
        let options = SearchLimits {
            white_time: 1_000,
            white_increment: 0,
            black_time: 10_000,
            black_increment: 0,
            ..SearchLimits::default()
        };

        let white_limits =
            compute_runtime_limits(options, &EngineOptions::default(), Color::White, 64);
        let black_limits =
            compute_runtime_limits(options, &EngineOptions::default(), Color::Black, 64);

        assert!(white_limits.soft_ms < black_limits.soft_ms);
        assert!(white_limits.hard_ms < black_limits.hard_ms);
    }

    #[test]
    fn movestogo_uses_explicit_clock_horizon() {
        let options = SearchLimits {
            black_time: 10_000,
            black_increment: 0,
            movestogo: 10,
            ..SearchLimits::default()
        };
        let limits = compute_runtime_limits(options, &EngineOptions::default(), Color::Black, 64);

        assert!((980.0..=1_000.0).contains(&limits.soft_ms));
        assert!(limits.hard_ms <= 6_000.0);
    }

    #[test]
    fn depth_is_clamped_and_absent_clock_is_unbounded() {
        let shallow = SearchLimits {
            depth: 0.25,
            ..SearchLimits::default()
        };
        let shallow_limits =
            compute_runtime_limits(shallow, &EngineOptions::default(), Color::White, 64);

        assert_eq!(shallow_limits.depth, 1);
        assert!(shallow_limits.soft_ms.is_infinite());
        assert!(shallow_limits.hard_ms.is_infinite());

        let unlimited = SearchLimits::default();
        let unlimited_limits =
            compute_runtime_limits(unlimited, &EngineOptions::default(), Color::Black, 42);

        assert_eq!(unlimited_limits.depth, 42);
    }
}
