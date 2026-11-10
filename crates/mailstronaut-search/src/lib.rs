//! Typo-tolerant local mail search.
//!
//! The index is derived data: SQLite stays the source of truth, and this directory can be
//! deleted and rebuilt at any time. It holds terms and identifiers rather than message
//! text that could be read back, and nothing here touches a network.
//!
//! Results come in tiers, newest first within each: whole words, then word beginnings.
//! Only when neither finds anything are typos corrected, because a correctly spelled word
//! should not bring back everything that merely looks like it. Relevance scoring is not
//! used, because recency predicts what a person wants in mail better than term frequency.

use core::{error::Error as StdError, fmt};
use std::{collections::BTreeMap, fs, ops::Bound, path::Path, sync::Mutex};

use tantivy::{
    DocAddress, Index as Engine, IndexReader, IndexWriter, Order, ReloadPolicy, Searcher,
    TantivyDocument, TantivyError, Term,
    collector::TopDocs,
    columnar::Column,
    directory::MmapDirectory,
    query::{
        BooleanQuery, FuzzyTermQuery, Occur, PhraseQuery, Query, RangeQuery, RegexQuery, TermQuery,
    },
    schema::{
        FAST, Field, INDEXED, IndexRecordOption, STRING, Schema, TextFieldIndexing, TextOptions,
    },
    tokenizer::{AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer},
};

const ANALYZER: &str = "mail";
/// Tantivy's minimum for one indexing thread, which is also this index's memory budget.
const WRITER_MEMORY: usize = 15_000_000;
const MAX_QUERY_BYTES: usize = 1024;
const MAX_QUERY_TERMS: usize = 32;

/// A redacted index failure: neither the query nor any mail text appears here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    /// The query is too long or contains control characters.
    InvalidQuery,
    /// Another process holds the index for writing.
    Busy,
    /// The index could not be read or written.
    Storage,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidQuery => "search query is too long or contains control characters",
            Self::Busy => "another process is using the search index",
            Self::Storage => "search index is unavailable",
        })
    }
}
impl StdError for Error {}
impl From<TantivyError> for Error {
    fn from(error: TantivyError) -> Self {
        if matches!(error, TantivyError::LockFailure(..)) {
            Self::Busy
        } else {
            Self::Storage
        }
    }
}

/// One message's searchable text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    /// Canonical message ID in the store.
    pub id: i64,
    /// Owning account, for isolating results.
    pub account: String,
    /// When the message arrived, for ordering results.
    pub received_at: i64,
    /// Subject line.
    pub subject: String,
    /// Parsed From names and addresses, searched by `from:`.
    pub sender: String,
    /// Every address on the message.
    pub addresses: String,
    /// Readable body text.
    pub body: String,
}

/// Original-level restrictions, evaluated before the result limit.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// Inclusive UTC arrival instant.
    pub received_from: Option<i64>,
    /// Exclusive UTC arrival instant.
    pub received_before: Option<i64>,
    /// Literal sender substring, ASCII case-insensitive.
    pub sender: String,
}

/// One change to apply to the index.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Change {
    /// Add or replace this message.
    Put(Document),
    /// Remove this message's terms.
    Remove(i64),
}

#[derive(Clone, Copy)]
struct Fields {
    id: Field,
    received_at: Field,
    account: Field,
    subject: Field,
    sender: Field,
    sender_literal: Field,
    addresses: Field,
    body: Field,
}
impl Fields {
    const fn text(self) -> [Field; 4] {
        [self.subject, self.sender, self.addresses, self.body]
    }
}

fn schema() -> (Schema, Fields) {
    let mut schema = Schema::builder();
    let text = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(ANALYZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    let fields = Fields {
        id: schema.add_i64_field("id", INDEXED | FAST),
        received_at: schema.add_i64_field("received_at", FAST),
        account: schema.add_text_field("account", STRING),
        subject: schema.add_text_field("subject", text.clone()),
        sender: schema.add_text_field("sender", text.clone()),
        sender_literal: schema.add_text_field("sender_literal", STRING),
        addresses: schema.add_text_field("addresses", text.clone()),
        body: schema.add_text_field("body", text),
    };
    (schema.build(), fields)
}

/// One analyzer for every language: lowercase and accent folding, with no stemming, so
/// that "Réinscription" and "reinscription" are the same term.
fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(64))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

fn tokens(text: &str) -> Vec<String> {
    let mut analyzer = analyzer();
    let mut stream = analyzer.token_stream(text);
    let mut tokens = Vec::new();
    while let Some(token) = stream.next() {
        tokens.push(token.text.clone());
    }
    tokens
}

/// How far a word may be from what is in a message.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Tier {
    Whole,
    Beginning,
    Typo,
}

/// One clause of a query. Exact parts come from quotes and never expand.
struct Part {
    tokens: Vec<String>,
    exact: bool,
    fields: Vec<Field>,
}

/// An opened index, and whether it holds nothing yet and needs filling from the store.
pub struct Opened {
    /// The usable index.
    pub index: Index,
    /// True when created just now, including after discarding an unusable one.
    pub fresh: bool,
}

/// The on-disk index. All methods block, so call them off any async executor thread.
pub struct Index {
    fields: Fields,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
}

impl Index {
    /// Open or create the index. An unusable one is rebuildable by definition, so it is
    /// discarded and recreated, while an index in use by another process is left alone.
    ///
    /// # Errors
    /// Returns [`Error::Busy`] if another process is writing, else [`Error::Storage`].
    pub fn open(directory: &Path) -> Result<Opened, Error> {
        match Self::try_open(directory) {
            Err(Error::Storage) => {
                match fs::remove_dir_all(directory) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(Error::Storage),
                }
                Self::try_open(directory)
            }
            result => result,
        }
    }

    fn try_open(directory: &Path) -> Result<Opened, Error> {
        fs::create_dir_all(directory).map_err(|_| Error::Storage)?;
        let storage = MmapDirectory::open(directory).map_err(|_| Error::Storage)?;
        let fresh = !Engine::exists(&storage).map_err(|_| Error::Storage)?;
        let (schema, fields) = schema();
        let engine = Engine::open_or_create(storage, schema)?;
        engine.tokenizers().register(ANALYZER, analyzer());
        let writer = engine.writer_with_num_threads(1, WRITER_MEMORY)?;
        let reader = engine
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Opened {
            index: Self {
                fields,
                reader,
                writer: Mutex::new(writer),
            },
            fresh,
        })
    }

    /// Apply changes as one commit, so that an interrupted batch leaves the last good
    /// state and can simply be applied again.
    ///
    /// # Errors
    /// Returns [`Error::Storage`] if the batch could not be committed.
    pub fn apply(&self, changes: Vec<Change>) -> Result<(), Error> {
        let mut writer = self.writer.lock().map_err(|_| Error::Storage)?;
        for change in changes {
            let id = match &change {
                Change::Put(document) => document.id,
                Change::Remove(id) => *id,
            };
            writer.delete_term(Term::from_field_i64(self.fields.id, id));
            if let Change::Put(document) = change
                && let Err(error) = writer.add_document(self.document(document))
            {
                _ = writer.rollback();
                return Err(error.into());
            }
        }
        if let Err(error) = writer.commit() {
            _ = writer.rollback();
            return Err(error.into());
        }
        drop(writer);
        self.reader.reload().map_err(Error::from)
    }

    fn document(&self, document: Document) -> TantivyDocument {
        let mut indexed = TantivyDocument::default();
        indexed.add_i64(self.fields.id, document.id);
        indexed.add_i64(self.fields.received_at, document.received_at);
        indexed.add_text(self.fields.account, document.account);
        indexed.add_text(self.fields.subject, document.subject);
        indexed.add_text(
            self.fields.sender_literal,
            document.sender.to_ascii_lowercase(),
        );
        indexed.add_text(self.fields.sender, document.sender);
        indexed.add_text(self.fields.addresses, document.addresses);
        indexed.add_text(self.fields.body, document.body);
        indexed
    }

    /// Message IDs matching a query in these accounts, best tier first and newest first
    /// within it. Quotes make an exact phrase, and `from:` and `subject:` restrict a part.
    /// Word beginnings are searched too, and typos are corrected only if nothing else matched.
    ///
    /// # Errors
    /// Rejects oversized queries and control characters, and reports storage failures.
    pub fn search(
        &self,
        input: &str,
        accounts: &[String],
        limit: usize,
    ) -> Result<Vec<i64>, Error> {
        self.search_filtered(input, accounts, limit, &Filter::default())
    }

    /// Search with literal sender and half-open arrival bounds before limiting hits.
    /// # Errors
    /// Rejects invalid bounds, oversized input and storage failures.
    pub fn search_filtered(
        &self,
        input: &str,
        accounts: &[String],
        limit: usize,
        filter: &Filter,
    ) -> Result<Vec<i64>, Error> {
        if filter.sender.len() > 256
            || filter.sender.chars().any(char::is_control)
            || matches!((filter.received_from, filter.received_before), (Some(from), Some(before)) if from >= before)
        {
            return Err(Error::InvalidQuery);
        }
        let parts = self.parts(input)?;
        if (parts.is_empty()
            && filter.sender.is_empty()
            && filter.received_from.is_none()
            && filter.received_before.is_none())
            || accounts.is_empty()
            || limit == 0
        {
            return Ok(Vec::new());
        }
        let searcher = self.reader.searcher();
        let mut hits = Vec::new();
        for tier in [Tier::Whole, Tier::Beginning, Tier::Typo] {
            // An all-exact query has only one reading, so one pass answers it.
            if tier != Tier::Whole && parts.iter().all(|part| part.exact) {
                break;
            }
            if tier == Tier::Typo && !hits.is_empty() {
                break;
            }
            let mut clauses: Vec<(Occur, Box<dyn Query>)> =
                vec![(Occur::Must, Box::new(self.query(&parts, accounts, tier)))];
            if filter.received_from.is_some() || filter.received_before.is_some() {
                let lower = filter.received_from.map_or(Bound::Unbounded, |value| {
                    Bound::Included(Term::from_field_i64(self.fields.received_at, value))
                });
                let upper = filter.received_before.map_or(Bound::Unbounded, |value| {
                    Bound::Excluded(Term::from_field_i64(self.fields.received_at, value))
                });
                clauses.push((Occur::Must, Box::new(RangeQuery::new(lower, upper))));
            }
            if !filter.sender.is_empty() {
                let pattern = format!(".*{}.*", regex::escape(&filter.sender.to_ascii_lowercase()));
                clauses.push((
                    Occur::Must,
                    Box::new(
                        RegexQuery::from_pattern(&pattern, self.fields.sender_literal)
                            .map_err(|_| Error::InvalidQuery)?,
                    ),
                ));
            }
            let query = BooleanQuery::new(clauses);
            for id in Self::newest(&searcher, &query, limit)? {
                if hits.len() < limit && !hits.contains(&id) {
                    hits.push(id);
                }
            }
            if hits.len() >= limit {
                break;
            }
        }
        Ok(hits)
    }

    fn newest(searcher: &Searcher, query: &dyn Query, limit: usize) -> Result<Vec<i64>, Error> {
        let order = TopDocs::with_limit(limit).order_by_fast_field("received_at", Order::Desc);
        let found: Vec<(Option<i64>, DocAddress)> = searcher.search(query, &order)?;
        let mut columns: BTreeMap<u32, Column<i64>> = BTreeMap::new();
        let mut ids = Vec::with_capacity(found.len());
        for (_, address) in found {
            let column = if let Some(column) = columns.get(&address.segment_ord) {
                column.clone()
            } else {
                let column = searcher
                    .segment_reader(address.segment_ord)
                    .fast_fields()
                    .i64("id")?;
                columns.insert(address.segment_ord, column.clone());
                column
            };
            if let Some(id) = column.first(address.doc_id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    fn parts(&self, input: &str) -> Result<Vec<Part>, Error> {
        if input.len() > MAX_QUERY_BYTES || input.chars().any(char::is_control) {
            return Err(Error::InvalidQuery);
        }
        let mut parts = Vec::new();
        let mut rest = input.trim_start();
        while !rest.is_empty() {
            let (fields, after) = self.scope(rest);
            let quoted = after
                .strip_prefix('"')
                .and_then(|quoted| quoted.split_once('"'));
            let (text, exact, tail) = if let Some((phrase, tail)) = quoted {
                (phrase, true, tail)
            } else {
                let end = after.find(char::is_whitespace).unwrap_or(after.len());
                (&after[..end], false, &after[end..])
            };
            let tokens = tokens(text);
            if exact && !tokens.is_empty() {
                parts.push(Part {
                    tokens,
                    exact,
                    fields,
                });
            } else {
                // An unquoted "foo-bar" is two words, each needing to match.
                parts.extend(tokens.into_iter().map(|token| Part {
                    tokens: vec![token],
                    exact,
                    fields: fields.clone(),
                }));
            }
            rest = tail.trim_start();
        }
        if parts.iter().map(|part| part.tokens.len()).sum::<usize>() > MAX_QUERY_TERMS {
            return Err(Error::InvalidQuery);
        }
        Ok(parts)
    }

    fn scope<'a>(&self, text: &'a str) -> (Vec<Field>, &'a str) {
        let lowered = |prefix: &str| {
            text.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        };
        if lowered("from:") {
            (vec![self.fields.sender], &text["from:".len()..])
        } else if lowered("subject:") {
            (vec![self.fields.subject], &text["subject:".len()..])
        } else {
            (self.fields.text().to_vec(), text)
        }
    }

    fn query(&self, parts: &[Part], accounts: &[String], tier: Tier) -> BooleanQuery {
        let accounts = accounts
            .iter()
            .map(|account| {
                let term = Term::from_field_text(self.fields.account, account);
                let query: Box<dyn Query> =
                    Box::new(TermQuery::new(term, IndexRecordOption::Basic));
                (Occur::Should, query)
            })
            .collect();
        let mut clauses: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, Box::new(BooleanQuery::new(accounts)))];
        clauses.extend(parts.iter().map(|part| {
            let matches = part
                .fields
                .iter()
                .map(|field| (Occur::Should, Self::clause(*field, part, tier)))
                .collect();
            let query: Box<dyn Query> = Box::new(BooleanQuery::new(matches));
            (Occur::Must, query)
        }));
        BooleanQuery::new(clauses)
    }

    fn clause(field: Field, part: &Part, tier: Tier) -> Box<dyn Query> {
        let term = |text: &str| Term::from_field_text(field, text);
        if part.tokens.len() > 1 {
            return Box::new(PhraseQuery::new(
                part.tokens.iter().map(|text| term(text)).collect(),
            ));
        }
        let word = part.tokens.first().map_or("", String::as_str);
        match tier {
            _ if part.exact => Box::new(TermQuery::new(term(word), IndexRecordOption::Basic)),
            Tier::Whole => Box::new(TermQuery::new(term(word), IndexRecordOption::Basic)),
            Tier::Beginning => Box::new(FuzzyTermQuery::new_prefix(term(word), 0, true)),
            Tier::Typo => Box::new(FuzzyTermQuery::new_prefix(
                term(word),
                allowed_typos(word),
                true,
            )),
        }
    }
}

/// Short words have too many neighbors: at one or two letters, a single edit matches
/// almost anything. Tantivy's fuzzy automaton supports at most two.
fn allowed_typos(word: &str) -> u8 {
    match word.chars().count() {
        0..=2 => 0,
        3..=5 => 1,
        _ => 2,
    }
}
