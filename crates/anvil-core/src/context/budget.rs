//! Token budgeting (SPEC §17). Heuristic: ~4 chars per token.

/// Rough token estimate for text.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Take items from highest priority until the budget is exhausted.
/// Returns the kept prefix and the number dropped.
pub fn fit_budget<T>(mut items: Vec<(T, usize)>, max_tokens: usize) -> (Vec<T>, usize) {
    let mut kept = Vec::new();
    let mut used = 0;
    let mut dropped = 0;
    for (item, cost) in items.drain(..) {
        if used + cost <= max_tokens {
            used += cost;
            kept.push(item);
        } else {
            dropped += 1;
        }
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_is_chars_over_four() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    #[test]
    fn fit_keeps_priority_order() {
        let (kept, dropped) = fit_budget(vec![("a", 5), ("b", 5), ("c", 5)], 10);
        assert_eq!(kept, vec!["a", "b"]);
        assert_eq!(dropped, 1);
    }
}
