//! Bounded literal context from the same canonical originals that a search opens.

use std::collections::BTreeMap;

use rusqlite::OptionalExtension as _;

use super::{Error, Store};

impl Store {
    /// Read literal body excerpts for a page of already scoped search hits.
    /// No match yields no excerpt; the normal preview remains available.
    /// # Errors
    /// Rejects oversized inputs and reports storage failures.
    pub fn body_excerpts(
        &self,
        originals: &[i64],
        words: &[String],
    ) -> Result<BTreeMap<i64, String>, Error> {
        if originals.len() > 100
            || words.len() > 32
            || words.iter().any(|word| {
                word.is_empty() || word.len() > 256 || word.chars().any(char::is_control)
            })
        {
            return Err(Error::InvalidInput);
        }
        let mut excerpts = BTreeMap::new();
        if originals.is_empty() || words.is_empty() {
            return Ok(excerpts);
        }
        let terms = words
            .iter()
            .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let expression = format!("body : ({terms})");
        // An original is already chosen, so a different member of its conversation
        // can neither supply its excerpt nor change which message opens.
        let mut statement = self.connection.prepare_cached(
            "SELECT substr(snippet(message_search, 2, '', '', '…', 14), 1, 480)
             FROM message_search WHERE rowid=?1 AND message_search MATCH ?2",
        )?;
        for &id in originals {
            let text: Option<String> = statement
                .query_row((id, &expression), |row| row.get(0))
                .optional()?;
            if let Some(text) = text {
                excerpts.insert(id, text.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
        Ok(excerpts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_excerpt_comes_from_the_opened_original_even_beyond_its_preview() {
        let root = tempfile::tempdir().expect("private profile");
        let store = Store::open(root.path()).expect("mail store");
        let body = format!(
            "{}\nRéinscription en école doctorale · date limite.",
            "Préambule artificiel. ".repeat(100)
        );
        store
            .connection
            .execute(
                "INSERT INTO message_search(rowid, subject, addresses, body) VALUES (1,'Notice','',?1), (2,'Réinscription','', 'Subject match only.')",
                [&body],
            )
            .expect("synthetic originals");
        let excerpts = store
            .body_excerpts(&[1, 2, 3], &["reinscription".into()])
            .expect("scoped excerpts");
        assert_eq!(excerpts.len(), 1);
        let matched = &excerpts[&1];
        assert!(matched.contains("Réinscription en école doctorale"));
        assert!(matched.chars().count() <= 480);
        assert!(!matched.contains('\n'));
    }
}
