//! Cost-control masking: the defaults and where the masked region ends.
//!
//! Masking rewrites history, and a provider caches a prompt only up to its
//! first change. Moving the masked region forward by one tool result on every
//! call rewrote the history on nearly every request of a busy session, so
//! everything after the newest masked result was read again at full price. The
//! region now moves in batches: it advances to a multiple of
//! `mask_batch_results`, or all the way when the results waiting past it pass
//! `mask_batch_bytes`. Results waiting in between stay verbatim and cached,
//! which costs a tenth of the input price on most providers.
//!
//! The boundary is a function of the history alone, replayed result by
//! result, so it only ever moves forward and a worker restart computes the
//! same one.

use super::CostControlConfig;

/// How many of the leading masking candidates may be masked: replays the
/// policy one tool result at a time and keeps the furthest boundary reached, so
/// a result once masked stays masked. `sizes` are the candidates' payload sizes
/// in history order.
pub(super) fn mask_boundary(sizes: &[usize], config: &CostControlConfig) -> usize {
    let keep = config.keep_recent_tool_results;
    let step = config.mask_batch_results.max(1);
    let mut boundary = 0;
    for seen in keep + 1..=sizes.len() {
        let maskable = seen - keep;
        boundary = boundary.max(maskable / step * step);
        let waiting: usize = sizes[boundary..maskable]
            .iter()
            .filter(|size| **size >= config.min_masked_tool_result_bytes)
            .sum();
        if waiting > config.mask_batch_bytes {
            boundary = maskable;
        }
    }
    boundary
}

pub(super) fn default_cost_control_mask_batch_results() -> usize {
    8
}

pub(super) fn default_cost_control_mask_batch_bytes() -> usize {
    64 * 1024
}

pub(super) fn default_cost_control_enabled() -> bool {
    true
}

pub(super) fn default_cost_control_keep_recent_tool_results() -> usize {
    2
}

pub(super) fn default_cost_control_mask_after_tool_results() -> usize {
    4
}

pub(super) fn default_cost_control_max_live_tool_result_bytes() -> usize {
    24 * 1024
}

// Masking a small result saves a few hundred tokens but takes it out of the
// model's view while it may still be working with it, and the model fetches it
// again. With the count trigger (4 results, keep 2) an agent juggling a pull
// request, its diff and a shell command re-fetched them in an endless cycle.
// Below this size the re-fetch costs more than keeping the result.
pub(super) fn default_cost_control_min_masked_tool_result_bytes() -> usize {
    4 * 1024
}

pub(super) fn default_cost_control_max_uncached_input_tokens() -> u32 {
    100_000
}

pub(super) fn default_cost_control_min_cache_read_ratio() -> f32 {
    0.35
}

pub(super) fn default_cost_control_compact_after_tool_result_bytes() -> usize {
    256 * 1024
}

pub(super) fn default_cost_control_compact_min_input_tokens() -> usize {
    8 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(step: usize, batch_bytes: usize) -> CostControlConfig {
        CostControlConfig {
            keep_recent_tool_results: 2,
            min_masked_tool_result_bytes: 4 * 1024,
            mask_batch_results: step,
            mask_batch_bytes: batch_bytes,
            ..CostControlConfig::default()
        }
    }

    #[test]
    fn the_boundary_moves_in_batches_and_never_back() {
        let config = config(8, usize::MAX);
        let mut previous = 0;
        let mut moves = 0;
        for count in 0..=30 {
            let boundary = mask_boundary(&vec![5 * 1024; count], &config);
            assert!(boundary >= previous, "unmasked a result at {count}");
            assert_eq!(boundary % 8, 0, "boundary {boundary} at {count}");
            moves += usize::from(boundary != previous);
            previous = boundary;
        }
        // 28 maskable results move the boundary three times, not 28.
        assert_eq!((previous, moves), (24, 3));
    }

    #[test]
    fn a_batch_of_one_masks_everything_but_the_recent_results() {
        assert_eq!(mask_boundary(&[5 * 1024; 7], &config(1, usize::MAX)), 5);
        assert_eq!(mask_boundary(&[5 * 1024; 2], &config(1, usize::MAX)), 0);
    }

    #[test]
    fn large_waiting_results_move_the_boundary_early_and_small_ones_do_not() {
        let config = config(8, 64 * 1024);
        // Two 40 KiB results waiting past the boundary exceed 64 KiB.
        assert_eq!(mask_boundary(&[40 * 1024; 4], &config), 2);
        assert_eq!(mask_boundary(&[40 * 1024; 3], &config), 0);
        // Results under the masking floor never count toward the batch.
        assert_eq!(mask_boundary(&[1024; 40], &config), 32);
        let mut sizes = vec![1024; 5];
        sizes.push(70 * 1024);
        sizes.extend([1024, 1024]);
        assert_eq!(mask_boundary(&sizes, &config), 6);
    }
}
