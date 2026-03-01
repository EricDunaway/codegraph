//! Name matching utilities for reference resolution

use codegraph_types::BUILTIN_SYMBOLS;
use std::collections::HashSet;

/// Name matcher for finding symbol candidates
pub struct NameMatcher {
    /// Set of built-in symbols to skip
    builtin_symbols: HashSet<&'static str>,
}

impl Default for NameMatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl NameMatcher {
    /// Create a new name matcher
    pub fn new() -> Self {
        let builtin_symbols: HashSet<&'static str> = BUILTIN_SYMBOLS.iter().copied().collect();
        Self { builtin_symbols }
    }

    /// Check if a symbol name is a built-in that should be skipped
    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtin_symbols.contains(name)
    }

    /// Check for exact name match
    pub fn exact_match(&self, name: &str, candidate: &str) -> bool {
        name == candidate
    }

    /// Check for case-insensitive match
    pub fn case_insensitive_match(&self, name: &str, candidate: &str) -> bool {
        name.eq_ignore_ascii_case(candidate)
    }

    /// Check if candidate matches as a qualified name suffix
    /// e.g., "foo::bar::baz" matches "baz" or "bar::baz"
    pub fn qualified_suffix_match(&self, name: &str, candidate: &str) -> bool {
        if name == candidate {
            return true;
        }

        // Check if candidate ends with the name after a separator
        for sep in ["::", ".", "/", "\\"] {
            if candidate.ends_with(&format!("{}{}", sep, name)) {
                return true;
            }
        }

        false
    }

    /// Calculate similarity score between two names (0.0 to 1.0)
    pub fn similarity(&self, a: &str, b: &str) -> f64 {
        if a == b {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        // Case insensitive exact match
        if a.eq_ignore_ascii_case(b) {
            return 0.95;
        }

        // Qualified suffix match
        if self.qualified_suffix_match(a, b) || self.qualified_suffix_match(b, a) {
            return 0.9;
        }

        // Levenshtein-based similarity for fuzzy matching
        let distance = self.levenshtein_distance(a, b);
        let max_len = a.len().max(b.len()) as f64;
        let similarity = 1.0 - (distance as f64 / max_len);

        // Threshold: require at least 70% similarity
        if similarity >= 0.7 {
            similarity * 0.8 // Scale down fuzzy matches
        } else {
            0.0
        }
    }

    /// Compute Levenshtein edit distance
    fn levenshtein_distance(&self, a: &str, b: &str) -> usize {
        let a_chars: Vec<char> = a.chars().collect();
        let b_chars: Vec<char> = b.chars().collect();
        let m = a_chars.len();
        let n = b_chars.len();

        if m == 0 {
            return n;
        }
        if n == 0 {
            return m;
        }

        let mut prev: Vec<usize> = (0..=n).collect();
        let mut curr = vec![0; n + 1];

        for i in 1..=m {
            curr[0] = i;
            for j in 1..=n {
                let cost = if a_chars[i - 1] == b_chars[j - 1] {
                    0
                } else {
                    1
                };
                curr[j] = (prev[j] + 1)
                    .min(curr[j - 1] + 1)
                    .min(prev[j - 1] + cost);
            }
            std::mem::swap(&mut prev, &mut curr);
        }

        prev[n]
    }

    /// Find the best match from a list of candidates
    pub fn find_best_match<'a>(
        &self,
        name: &str,
        candidates: &[&'a str],
    ) -> Option<(&'a str, f64)> {
        if self.is_builtin(name) {
            return None;
        }

        let mut best: Option<(&'a str, f64)> = None;

        for &candidate in candidates {
            let score = self.similarity(name, candidate);
            if score > 0.0 {
                match &best {
                    None => best = Some((candidate, score)),
                    Some((_, best_score)) if score > *best_score => {
                        best = Some((candidate, score));
                    }
                    _ => {}
                }
            }
        }

        best
    }

    /// Find all matches above a threshold
    pub fn find_all_matches<'a>(
        &self,
        name: &str,
        candidates: &[&'a str],
        threshold: f64,
    ) -> Vec<(&'a str, f64)> {
        if self.is_builtin(name) {
            return Vec::new();
        }

        let mut matches: Vec<(&'a str, f64)> = candidates
            .iter()
            .filter_map(|&candidate| {
                let score = self.similarity(name, candidate);
                if score >= threshold {
                    Some((candidate, score))
                } else {
                    None
                }
            })
            .collect();

        // Sort by score descending
        matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        matches
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_detection() {
        let matcher = NameMatcher::new();

        assert!(matcher.is_builtin("console"));
        assert!(matcher.is_builtin("window"));
        assert!(matcher.is_builtin("Promise"));
        assert!(matcher.is_builtin("print")); // Python
        assert!(matcher.is_builtin("React")); // React

        assert!(!matcher.is_builtin("myFunction"));
        assert!(!matcher.is_builtin("UserService"));
    }

    #[test]
    fn test_exact_match() {
        let matcher = NameMatcher::new();

        assert!(matcher.exact_match("foo", "foo"));
        assert!(!matcher.exact_match("foo", "Foo"));
        assert!(!matcher.exact_match("foo", "bar"));
    }

    #[test]
    fn test_case_insensitive_match() {
        let matcher = NameMatcher::new();

        assert!(matcher.case_insensitive_match("foo", "foo"));
        assert!(matcher.case_insensitive_match("foo", "FOO"));
        assert!(matcher.case_insensitive_match("FooBar", "foobar"));
        assert!(!matcher.case_insensitive_match("foo", "bar"));
    }

    #[test]
    fn test_qualified_suffix_match() {
        let matcher = NameMatcher::new();

        assert!(matcher.qualified_suffix_match("baz", "foo::bar::baz"));
        assert!(matcher.qualified_suffix_match("bar::baz", "foo::bar::baz"));
        assert!(matcher.qualified_suffix_match("baz", "foo.bar.baz"));
        assert!(!matcher.qualified_suffix_match("bar", "foo::baz::qux"));
    }

    #[test]
    fn test_similarity() {
        let matcher = NameMatcher::new();

        // Exact match
        assert!((matcher.similarity("foo", "foo") - 1.0).abs() < 0.001);

        // Case insensitive
        assert!(matcher.similarity("foo", "FOO") > 0.9);

        // Qualified suffix
        assert!(matcher.similarity("baz", "foo::baz") > 0.85);

        // Similar names (typo)
        assert!(matcher.similarity("getUserById", "getUserByld") > 0.7);

        // Different names
        assert!(matcher.similarity("foo", "bar") < 0.5);
    }

    #[test]
    fn test_find_best_match() {
        let matcher = NameMatcher::new();
        let candidates = vec!["getUserById", "getUser", "getUserList", "deleteUser"];

        let result = matcher.find_best_match("getUserById", &candidates);
        assert!(result.is_some());
        let (name, score) = result.unwrap();
        assert_eq!(name, "getUserById");
        assert!((score - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_find_best_match_skips_builtins() {
        let matcher = NameMatcher::new();
        let candidates = vec!["console", "myConsole", "logger"];

        // Looking for "console" should return None (it's a builtin)
        let result = matcher.find_best_match("console", &candidates);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_all_matches() {
        let matcher = NameMatcher::new();
        let candidates = vec!["getUser", "getUserById", "getUserList", "deleteUser"];

        let matches = matcher.find_all_matches("getUser", &candidates, 0.5);

        // Should find at least "getUser" as exact match
        assert!(!matches.is_empty());
        assert_eq!(matches[0].0, "getUser");
        assert!((matches[0].1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_levenshtein_distance() {
        let matcher = NameMatcher::new();

        assert_eq!(matcher.levenshtein_distance("", ""), 0);
        assert_eq!(matcher.levenshtein_distance("abc", ""), 3);
        assert_eq!(matcher.levenshtein_distance("", "abc"), 3);
        assert_eq!(matcher.levenshtein_distance("abc", "abc"), 0);
        assert_eq!(matcher.levenshtein_distance("abc", "abd"), 1);
        assert_eq!(matcher.levenshtein_distance("kitten", "sitting"), 3);
    }
}
