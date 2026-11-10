//! Fuzzy mail search over synthetic messages in several languages.

use mailstronaut_search::{Change, Document, Error, Index};

#[test]
fn metadata_restrictions_precede_the_hit_limit_and_treat_sender_metacharacters_literally() {
    let (_root, index) = fixture();
    let mut old = document(100, "work", 10, "Budget", "Older relevant message");
    old.sender = "Name <a+b@example.test>".into();
    index.apply(vec![Change::Put(old)]).expect("index");
    let mut filter = mailstronaut_search::Filter {
        received_from: Some(10),
        received_before: Some(11),
        sender: "A+B@EXAMPLE.TEST".into(),
    };
    assert_eq!(
        index
            .search_filtered("budget", &["work".into()], 1, &filter)
            .expect("filter"),
        vec![100]
    );
    assert_eq!(
        index
            .search_filtered("", &["work".into()], 1, &filter)
            .expect("filter only"),
        vec![100]
    );
    filter.received_before = Some(10);
    assert_eq!(
        index.search_filtered("budget", &["work".into()], 1, &filter),
        Err(Error::InvalidQuery)
    );
    filter.received_from = None;
    filter.received_before = None;
    filter.sender = ".*".into();
    assert_eq!(
        index
            .search_filtered("budget", &["work".into()], 1, &filter)
            .expect("literal filter"),
        Vec::<i64>::new()
    );
}

fn document(id: i64, account: &str, received_at: i64, subject: &str, body: &str) -> Document {
    Document {
        id,
        account: account.into(),
        received_at,
        subject: subject.into(),
        sender: format!("Correspondent {id}"),
        addresses: format!("person{id}@example.test"),
        body: body.into(),
    }
}

fn corpus() -> Vec<Change> {
    [
        document(
            1,
            "work",
            100,
            "Réinscription en école doctorale",
            "Votre dossier de réinscription doit être déposé avant le 15 novembre.",
        ),
        document(
            2,
            "work",
            200,
            "Riunione del dipartimento",
            "La riunione è confermata per giovedì. Università di Torino.",
        ),
        document(
            3,
            "work",
            300,
            "Quarterly budget review",
            "Please review the attached budget before Friday.",
        ),
        document(
            4,
            "home",
            400,
            "Budget for the holidays",
            "Our holiday budget is almost spent.",
        ),
        document(
            5,
            "work",
            500,
            "Budgetary controls",
            "The budgetary controls were updated.",
        ),
    ]
    .into_iter()
    .map(Change::Put)
    .collect()
}

fn fixture() -> (tempfile::TempDir, Index) {
    let root = tempfile::tempdir().expect("fixture");
    let opened = Index::open(&root.path().join("search")).expect("open");
    assert!(opened.fresh);
    opened.index.apply(corpus()).expect("apply");
    (root, opened.index)
}

fn search(index: &Index, query: &str) -> Vec<i64> {
    index
        .search(query, &["work".to_owned(), "home".to_owned()], 100)
        .expect("search")
}

#[test]
fn accents_and_case_do_not_matter_in_either_direction() {
    let (_root, index) = fixture();
    for query in [
        "réinscription",
        "reinscription",
        "REINSCRIPTION",
        "Réinscription",
        "ecole",
        "école",
        "universita",
    ] {
        assert!(!search(&index, query).is_empty(), "{query}");
    }
    assert_eq!(search(&index, "reinscription"), [1]);
    assert_eq!(search(&index, "giovedi"), [2]);
}

#[test]
fn typos_are_forgiven_according_to_word_length() {
    let (_root, index) = fixture();
    for (query, expected) in [
        // One edit: a missing, a doubled, a wrong and a swapped letter.
        ("doctoale", vec![1]),
        ("dipartimennto", vec![2]),
        ("riunone", vec![2]),
        ("reivew", vec![3]),
        // Two edits on a long word.
        ("dipartmnto", vec![2]),
    ] {
        assert_eq!(search(&index, query), expected, "{query}");
    }
    // A correct word does not bring back its lookalikes.
    assert_eq!(search(&index, "person4"), [4]);
    // Nothing is close to this, and a short word gets no such tolerance.
    assert_eq!(search(&index, "zzzzzzzz"), Vec::<i64>::new());
    assert_eq!(search(&index, "ca"), Vec::<i64>::new());
}

#[test]
fn whole_words_come_before_beginnings_and_each_tier_is_newest_first() {
    let (_root, index) = fixture();
    // 5 is newest, but "budgetary" only begins with the word searched for.
    assert_eq!(search(&index, "budget"), [4, 3, 5]);
    assert_eq!(search(&index, "budge"), [5, 4, 3]);
}

#[test]
fn every_word_must_match_and_quotes_mean_exactly_that_phrase() {
    let (_root, index) = fixture();
    assert_eq!(search(&index, "quarterly friday"), [3]);
    assert_eq!(search(&index, "quarterly holidays"), Vec::<i64>::new());
    assert_eq!(search(&index, "\"holiday budget\""), [4]);
    assert_eq!(search(&index, "\"budget holiday\""), Vec::<i64>::new());
    // A quoted word is not expanded, so this typo finds nothing.
    assert_eq!(search(&index, "\"reivew\""), Vec::<i64>::new());
    assert_eq!(search(&index, "\"budge\""), Vec::<i64>::new());
}

#[test]
fn fields_can_be_named() {
    let (_root, index) = fixture();
    assert_eq!(search(&index, "subject:riunione"), [2]);
    assert_eq!(search(&index, "SUBJECT:friday"), Vec::<i64>::new());
    assert_eq!(search(&index, "from:correspondent subject:quarterly"), [3]);
    assert_eq!(search(&index, "person4"), [4]);
    assert_eq!(search(&index, "from:person4"), Vec::<i64>::new());
    assert_eq!(search(&index, "subject:\"budget review\""), [3]);
}

#[test]
fn accounts_are_isolated_and_no_account_means_no_results() {
    let (_root, index) = fixture();
    let hits = |accounts: &[&str]| {
        let accounts: Vec<String> = accounts.iter().map(|&name| name.to_owned()).collect();
        index.search("budget", &accounts, 100).expect("search")
    };
    assert_eq!(hits(&["work"]), [3, 5]);
    assert_eq!(hits(&["home"]), [4]);
    assert_eq!(hits(&["nobody"]), Vec::<i64>::new());
    assert_eq!(hits(&[]), Vec::<i64>::new());
}

#[test]
fn replacing_and_removing_leave_no_stale_terms_and_results_are_bounded() {
    let (_root, index) = fixture();
    index
        .apply(vec![
            Change::Put(document(3, "work", 300, "Renamed", "Nothing relevant.")),
            Change::Remove(4),
            Change::Remove(999),
        ])
        .expect("apply");
    assert_eq!(search(&index, "budget"), [5]);
    assert_eq!(search(&index, "renamed"), [3]);
    assert_eq!(search(&index, "holidays"), Vec::<i64>::new());
    let accounts = ["work".to_owned()];
    assert_eq!(index.search("the", &accounts, 1).expect("search").len(), 1);
    assert_eq!(
        index.search("the", &accounts, 0).expect("search"),
        Vec::<i64>::new()
    );
}

#[test]
fn hostile_queries_are_just_text() {
    let (_root, index) = fixture();
    for query in [
        "",
        "   ",
        "\"",
        "\"\"",
        "\"unterminated phrase",
        "AND OR NOT",
        "budget AND",
        "(((",
        "field:value:other",
        "from:",
        "subject:\"",
        "*",
        "budget*",
        "' ; DROP TABLE messages; --",
        "\\",
        "🦀",
        "日本語",
        "é",
        "ß",
    ] {
        search(&index, query);
    }
    assert_eq!(
        search(&index, "budget AND"),
        Vec::<i64>::new(),
        "AND is an ordinary word"
    );
    assert_eq!(search(&index, "budget*"), search(&index, "budget"));
    for invalid in [
        "x".repeat(1025),
        "line\nbreak".to_owned(),
        "nul\0byte".to_owned(),
        "word ".repeat(33),
    ] {
        assert_eq!(
            index.search(&invalid, &["work".to_owned()], 10),
            Err(Error::InvalidQuery)
        );
    }
}

#[test]
fn the_index_persists_and_a_damaged_one_is_rebuilt_while_a_busy_one_is_left_alone() {
    let root = tempfile::tempdir().expect("fixture");
    let path = root.path().join("search");
    {
        let opened = Index::open(&path).expect("open");
        assert!(opened.fresh);
        opened.index.apply(corpus()).expect("apply");
        // A second process must not be able to take over, or discard, an index in use.
        assert_eq!(Index::open(&path).err(), Some(Error::Busy));
        assert_eq!(search(&opened.index, "riunione"), [2]);
    }
    let opened = Index::open(&path).expect("reopen");
    assert!(!opened.fresh, "kept across a restart");
    assert_eq!(search(&opened.index, "riunione"), [2]);
    drop(opened);
    std::fs::write(path.join("meta.json"), b"not json").expect("damage");
    let opened = Index::open(&path).expect("recreated");
    assert!(opened.fresh, "the caller must refill it from the store");
    assert_eq!(search(&opened.index, "riunione"), Vec::<i64>::new());
}

fn fill_with_synthetic_mail(index: &Index, count: i64) {
    const WORDS: &[&str] = &[
        "budget",
        "réunion",
        "dossier",
        "università",
        "meeting",
        "deadline",
        "inscription",
        "rapport",
        "progetto",
        "review",
        "colloque",
        "seminario",
        "invoice",
        "calendrier",
        "laboratorio",
        "research",
        "document",
        "étudiant",
        "dottorato",
        "conference",
        "projet",
        "riunione",
        "schedule",
        "bibliothèque",
        "biblioteca",
        "committee",
        "thèse",
        "tesi",
    ];
    let word = |seed: u64| WORDS[usize::try_from(seed).expect("index") % WORDS.len()];
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut batch = Vec::new();
    for id in 0..count {
        let body: Vec<String> = (0..300)
            .map(|_| {
                let value = next();
                // Unique tokens keep the term dictionary realistic for fuzzy expansion.
                if value % 7 == 0 {
                    format!("{}{}", word(value >> 8), value % 40_000)
                } else {
                    word(value >> 8).to_owned()
                }
            })
            .collect();
        batch.push(Change::Put(Document {
            id,
            account: if id % 3 == 0 { "home" } else { "work" }.into(),
            received_at: id,
            subject: format!("{} {} {id}", word(next()), word(next())),
            sender: format!("Person {} <person{id}@example.test>", id % 500),
            addresses: format!("person{id}@example.test reader@example.test"),
            body: body.join(" "),
        }));
        if batch.len() == 64 {
            index.apply(core::mem::take(&mut batch)).expect("apply");
        }
    }
    index.apply(batch).expect("apply");
}

/// Measurements for the adoption decision. Not part of the normal run, and only meaningful
/// in a release build:
/// `cargo test --release -p mailstronaut-search --test search -- --ignored --nocapture`
#[test]
#[ignore = "explicit release-mode measurement"]
fn scale_measurement() {
    for count in [10_000_i64, 100_000] {
        let root = tempfile::tempdir().expect("fixture");
        let path = root.path().join("search");
        let index = Index::open(&path).expect("open").index;
        let began = std::time::Instant::now();
        fill_with_synthetic_mail(&index, count);
        let indexed = began.elapsed();
        let accounts = ["work".to_owned(), "home".to_owned()];
        for query in [
            "budget",
            "budgt",
            "reunoin",
            "progetto review",
            "\"budget review\"",
            "per",
        ] {
            let mut times: Vec<f64> = (0..30)
                .map(|_| {
                    let began = std::time::Instant::now();
                    index.search(query, &accounts, 1000).expect("search");
                    began.elapsed().as_secs_f64() * 1000.0
                })
                .collect();
            times.sort_by(f64::total_cmp);
            println!(
                "{count}: {query:?} p50 {:.1} ms, p95 {:.1} ms",
                times[15], times[28]
            );
        }
        let size: u64 = std::fs::read_dir(&path)
            .expect("index directory")
            .map(|entry| entry.expect("entry").metadata().expect("metadata").len())
            .sum();
        let status = std::fs::read_to_string("/proc/self/status").expect("status");
        let peak = status
            .lines()
            .find(|line| line.starts_with("VmHWM"))
            .unwrap_or_default();
        println!(
            "{count}: indexed in {:.1} s, {} MiB on disk, process {peak}",
            indexed.as_secs_f64(),
            size / 1024 / 1024
        );
    }
}
