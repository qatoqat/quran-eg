//! Small helpers.

/// "a, b, c (+N more)": at most `shown` items of a list, for banners.
pub fn short_list<T: ToString>(items: &[T], shown: usize) -> String {
    let mut s = items.iter().take(shown).map(T::to_string).collect::<Vec<_>>().join(", ");
    if items.len() > shown {
        s.push_str(&format!(" (+{} more)", items.len() - shown));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_list_truncates() {
        let keys: Vec<String> = (1..=14).map(|n| format!("2:{n}")).collect();
        assert_eq!(short_list(&keys[..2], 12), "2:1, 2:2");
        assert!(short_list(&keys, 12).ends_with("2:12 (+2 more)"));
        assert_eq!(short_list(&[3u16, 4], 1), "3 (+1 more)");
    }
}
