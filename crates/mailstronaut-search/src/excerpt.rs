//! Literal match ranges in an already selected original's displayed excerpt.

use std::ops::Range;

use tantivy::{schema::Field, snippet::SnippetGenerator};

use super::{MAX_QUERY_BYTES, MAX_QUERY_TERMS, analyzer, tokens};

/// Reuses the lexical analyzer to keep accent-folded matches at the original UTF-8 offsets.
pub struct BodyExcerpt {
    words: Vec<String>,
    generator: SnippetGenerator,
}

impl BodyExcerpt {
    /// Unquoted, unscoped word queries can look for literal terms in a body. Do not
    /// treat a sender/subject restriction or a phrase as a bag of body words.
    #[must_use]
    pub fn for_words(query: &str) -> Option<Self> {
        if query.len() > MAX_QUERY_BYTES
            || query.contains('"')
            || query.chars().any(char::is_control)
            || query.split_whitespace().any(|word| {
                let word = word.to_ascii_lowercase();
                word.starts_with("from:") || word.starts_with("subject:")
            })
        {
            return None;
        }
        let words = tokens(query);
        if words.is_empty() || words.len() > MAX_QUERY_TERMS {
            return None;
        }
        // The field number is unused by the text-only snippet API.
        let generator = SnippetGenerator::new(
            words.iter().map(|word| (word.clone(), 1.0)).collect(),
            analyzer(),
            Field::from_field_id(0),
            512,
        );
        Some(Self { words, generator })
    }

    /// Normalized literal terms for the store's scoped excerpt query.
    #[must_use]
    pub fn words(&self) -> &[String] {
        &self.words
    }

    /// Indices into the supplied text, without rewriting or normalizing the displayed source.
    #[must_use]
    pub fn highlights(&self, text: &str) -> Vec<Range<usize>> {
        let snippet = self.generator.snippet(text);
        let start = snippet.fragment_range().start;
        snippet
            .highlighted()
            .iter()
            .map(|range| start + range.start..start + range.end)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_folding_keeps_ranges_in_the_original_and_skips_near_matches() {
        let words = BodyExcerpt::for_words("reinscription boleto").expect("word query");
        let body = "… Réinscription en doctorat. Boletos et boleto.";
        let ranges = words.highlights(body);
        let marked = ranges
            .into_iter()
            .map(|range| &body[range])
            .collect::<Vec<_>>();
        assert_eq!(marked, ["Réinscription", "boleto"]);
        assert_eq!(
            words.highlights("Une inscription différente."),
            Vec::<Range<usize>>::new()
        );
    }

    #[test]
    fn constrained_queries_do_not_turn_into_unrelated_body_word_matches() {
        assert!(BodyExcerpt::for_words("\"exact phrase\"").is_none());
        assert!(BodyExcerpt::for_words("FROM:alice@example.test body").is_none());
        assert!(BodyExcerpt::for_words("subject:meeting").is_none());
        assert!(BodyExcerpt::for_words("  ").is_none());
    }
}
