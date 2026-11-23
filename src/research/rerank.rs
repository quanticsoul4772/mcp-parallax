//! Which fetched pages reach extraction after a rerank (061 research.md D3,
//! D7). Pure: scores in, indices out.

/// The indices of the `k` highest scores, in fetched (input) order. Equal
/// scores keep fetched order; with `k` at or above the count every index is
/// kept.
#[must_use]
pub fn keep_top(scores: &[f32], k: usize) -> Vec<usize> {
    let mut ranked: Vec<usize> = (0..scores.len()).collect();
    // Stable sort, highest first: ties stay in fetched order.
    ranked.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
    ranked.truncate(k);
    ranked.sort_unstable();
    ranked
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// 061 T003: the top `k` scores, returned in fetched order.
    #[test]
    fn the_top_k_come_back_in_fetched_order() {
        assert_eq!(keep_top(&[0.1, 0.9, 0.5, 0.7], 2), vec![1, 3]);
    }

    /// 061 T003 (D7): equal scores keep fetched order.
    #[test]
    fn ties_keep_fetched_order() {
        assert_eq!(keep_top(&[0.5, 0.5, 0.5], 2), vec![0, 1]);
        assert_eq!(keep_top(&[0.2, 0.8, 0.8, 0.8], 2), vec![1, 2]);
    }

    /// 061 T003 (FR-003): with `k` at or above the count, every page is kept.
    #[test]
    fn k_at_or_above_the_count_keeps_every_page() {
        assert_eq!(keep_top(&[0.3, 0.1], 2), vec![0, 1]);
        assert_eq!(keep_top(&[0.3, 0.1], 5), vec![0, 1]);
    }

    /// 061 T003: no scores keeps nothing.
    #[test]
    fn no_scores_keeps_nothing() {
        assert!(keep_top(&[], 3).is_empty());
    }
}
