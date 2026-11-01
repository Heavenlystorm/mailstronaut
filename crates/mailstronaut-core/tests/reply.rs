//! Synthetic reply routing, self-exclusion, ancestry and explicit text forwarding.

use mailstronaut_config::Account;
use mailstronaut_core::{
    mime,
    reply::{self, Kind},
};

fn account() -> Account {
    serde_json::from_value(serde_json::json!({"id":"receiving-account","provider":"manual","address":"me@example.test","aliases":["alias@example.test"],"displayName":"My Name","userName":"me","auth":{"kind":"password","secretRef":"fixture"}})).expect("account")
}
fn source(headers: &str) -> mime::Projection {
    mime::project(format!("{headers}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nCafé body.\r\nSecond line.\r\n").as_bytes()).expect("MIME")
}
#[test]
fn reply_prefers_reply_to_and_the_receiving_alias_and_preserves_thread_headers() {
    let source = source(
        "From: Author <author@example.test>\r\nReply-To: \"Desk, Team\" <desk@example.test>\r\nTo: My Alias <ALIAS@example.test>\r\nBcc: hidden@example.test\r\nSubject: Re: Questionnaire\r\nMessage-ID: <child@example.test>\r\nReferences: <root@example.test> <root@example.test>",
    );
    let draft = reply::prepare(&account(), &[], &source, Kind::Reply).expect("reply");
    assert_eq!(draft.account, "receiving-account");
    let frozen = draft.content.freeze().expect("valid reply MIME");
    let parsed = mime::project(frozen.original()).expect("independent MIME parser");
    assert_eq!(
        parsed.headers.from[0].address.as_deref(),
        Some("alias@example.test")
    );
    assert_eq!(
        parsed.headers.to[0].address.as_deref(),
        Some("desk@example.test")
    );
    assert_eq!(parsed.headers.to[0].name.as_deref(), Some("Desk, Team"));
    assert_eq!(parsed.headers.cc, []);
    assert_eq!(parsed.headers.bcc, []);
    assert_eq!(parsed.headers.subject.as_deref(), Some("Re: Questionnaire"));
    assert_eq!(parsed.headers.in_reply_to, vec!["child@example.test"]);
    assert_eq!(
        parsed.headers.references,
        vec!["root@example.test", "child@example.test"]
    );
    assert!(draft.content.body.contains("> Café body."));
    assert!(!String::from_utf8_lossy(frozen.original()).contains("hidden@example.test"));
}
#[test]
fn reply_all_excludes_all_self_identities_and_deduplicates_without_folding_other_local_parts() {
    let source = source(
        "From: Author <author@example.test>\r\nReply-To: Desk <desk@example.test>\r\nTo: me@example.test, desk@EXAMPLE.TEST, Case@example.test, case@example.test\r\nCc: ALIAS@example.test, Desk <desk@example.test>, colleague@example.test\r\nBcc: hidden@example.test\r\nSubject: Topic\r\nMessage-ID: <child@example.test>",
    );
    let draft = reply::prepare(&account(), &[], &source, Kind::ReplyAll).expect("reply all");
    let parsed = mime::project(draft.content.freeze().expect("MIME").original()).expect("parse");
    let to = parsed
        .headers
        .to
        .iter()
        .filter_map(|mailbox| mailbox.address.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(
        to,
        vec![
            "desk@example.test",
            "Case@example.test",
            "case@example.test"
        ]
    );
    assert_eq!(parsed.headers.cc.len(), 1);
    assert_eq!(
        parsed.headers.cc[0].address.as_deref(),
        Some("colleague@example.test")
    );
    assert_eq!(parsed.headers.bcc, []);
}
#[test]
fn replying_to_own_sent_mail_targets_original_recipients_and_keeps_the_sender_alias() {
    let source = source(
        "From: My Alias <alias@example.test>\r\nTo: colleague@example.test\r\nCc: other@example.test\r\nBcc: hidden@example.test\r\nSubject: Sent topic\r\nMessage-ID: <child@example.test>\r\nIn-Reply-To: <parent@example.test>",
    );
    let draft = reply::prepare(&account(), &[], &source, Kind::ReplyAll).expect("sent reply");
    assert_eq!(draft.content.from, "My Name <alias@example.test>");
    assert_eq!(draft.content.to, "colleague@example.test");
    assert_eq!(draft.content.cc, "other@example.test");
    assert_eq!(
        draft.content.references,
        vec!["parent@example.test", "child@example.test"]
    );
    assert_eq!(draft.content.bcc, "");
}
#[test]
fn text_forward_is_a_new_thread_and_does_not_leak_bcc_or_promise_attachments() {
    let source = source(
        "From: Author <author@example.test>\r\nTo: me@example.test\r\nCc: colleague@example.test\r\nBcc: hidden@example.test\r\nSubject: Fwd: Topic\r\nMessage-ID: <child@example.test>\r\nReferences: <root@example.test>",
    );
    let mut draft = reply::prepare(&account(), &[], &source, Kind::ForwardText).expect("forward");
    assert_eq!(draft.content.subject, "Fwd: Topic");
    assert_eq!(draft.content.to, "");
    assert_eq!(draft.content.cc, "");
    assert_eq!(draft.content.bcc, "");
    assert!(draft.content.in_reply_to.is_none());
    assert_eq!(draft.content.references, Vec::<String>::new());
    assert!(draft.content.body.contains("attachments not included"));
    assert!(!draft.content.body.contains("hidden@example.test"));
    draft.content.to = "recipient@example.test".into();
    let parsed = mime::project(draft.content.freeze().expect("MIME").original()).expect("parse");
    assert_eq!(parsed.headers.references, Vec::<String>::new());
    assert_eq!(parsed.headers.in_reply_to, Vec::<String>::new());
}
#[test]
fn malformed_incomplete_encrypted_and_oversized_input_never_creates_a_partial_quote() {
    let original = source("From: author@example.test\r\nTo: me@example.test\r\nSubject: Topic");
    let mut message = original.clone();
    message.status.encrypted_unsupported = true;
    assert!(matches!(
        reply::prepare(&account(), &[], &message, Kind::Reply),
        Err(reply::Error::Incomplete)
    ));
    message = original.clone();
    message.status.headers_truncated = true;
    assert!(matches!(
        reply::prepare(&account(), &[], &message, Kind::ReplyAll),
        Err(reply::Error::Incomplete)
    ));
    message = original.clone();
    message.headers.subject = Some("injected\r\nBcc: hidden@example.test".into());
    assert!(matches!(
        reply::prepare(&account(), &[], &message, Kind::Reply),
        Err(reply::Error::Addresses)
    ));
    message = original;
    message.text = Some("\n".repeat(mime::MAX_BODY_BYTES / 2));
    assert!(matches!(
        reply::prepare(&account(), &[], &message, Kind::Reply),
        Err(reply::Error::TooLarge)
    ));
}
#[test]
fn old_cached_header_json_defaults_the_new_reply_to_field() {
    let message = source("From: author@example.test\r\nTo: me@example.test");
    let mut old = serde_json::to_value(message.headers).expect("metadata");
    old.as_object_mut().expect("headers").remove("reply_to");
    let restored: mime::Headers =
        serde_json::from_value(old).expect("schema-compatible cached headers");
    assert_eq!(restored.reply_to, []);
}

#[test]
fn reply_all_excludes_other_configured_accounts_without_changing_the_receiving_sender() {
    let receiving = account();
    let mut other = account();
    other.id = "other".into();
    other.address = "other@example.test".into();
    other.aliases = vec!["other-alias@example.test".into()];
    let source=mime::project(b"From: author@example.test\r\nTo: alias@example.test, OTHER@example.test, colleague@example.test\r\nCc: other-alias@example.test, me@example.test\r\nMessage-ID: <source@example.test>\r\n\r\nSynthetic cross-account recipients").expect("projection");
    let draft = reply::prepare(
        &receiving,
        std::slice::from_ref(&other),
        &source,
        Kind::ReplyAll,
    )
    .expect("reply all");
    assert_eq!(
        draft.content.to,
        "author@example.test, colleague@example.test"
    );
    assert_eq!(draft.content.cc, "");
    assert!(draft.content.from.contains("alias@example.test"));
    assert!(!draft.content.from.contains("other@example.test"));
    let sent=mime::project(b"From: other@example.test\r\nTo: alias@example.test, colleague@example.test\r\nMessage-ID: <own-source@example.test>\r\n\r\nSynthetic other own account author").expect("projection");
    let draft =
        reply::prepare(&receiving, &[other], &sent, Kind::ReplyAll).expect("own-sent reply");
    assert_eq!(draft.content.to, "colleague@example.test");
    assert_eq!(draft.content.cc, "");
    assert!(draft.content.from.contains("alias@example.test"));
    assert_eq!(draft.account, "receiving-account");
}
#[test]
fn a_signature_sits_above_the_quote_and_reaches_the_frozen_message() {
    let source = source(
        "From: Author <author@example.test>\r\nTo: me@example.test\r\nSubject: Topic\r\nMessage-ID: <child@example.test>",
    );
    let mut signed = account();
    signed.signature = Some("My Name\nMy Laboratory".into());
    for kind in [Kind::Reply, Kind::ReplyAll, Kind::ForwardText] {
        let draft = reply::prepare(&signed, &[], &source, kind).expect("prepared");
        let body = &draft.content.body;
        assert!(
            body.starts_with("\n\n-- \nMy Name\nMy Laboratory\n\n"),
            "{body}"
        );
        assert!(body.contains("Café body."), "{body}");
    }
    let unsigned = reply::prepare(&account(), &[], &source, Kind::Reply).expect("prepared");
    assert!(!unsigned.content.body.contains("-- \n"));
}
#[test]
fn a_new_message_leaves_room_above_its_signature_and_no_signature_means_no_delimiter() {
    assert_eq!(reply::with_signature("", Some("Me")), "\n\n-- \nMe");
    assert_eq!(reply::with_signature("", None), "");
    assert_eq!(reply::with_signature("\n\nquote", None), "\n\nquote");
}
#[test]
fn moving_a_draft_swaps_an_untouched_signature_and_keeps_an_edited_one() {
    let body = reply::with_signature("\n\n> quote", Some("Work"));
    assert_eq!(
        reply::replace_signature(&body, Some("Work"), Some("Home")),
        "\n\n-- \nHome\n\n> quote"
    );
    assert_eq!(
        reply::replace_signature(&body, Some("Work"), None),
        "\n\n> quote"
    );
    let edited = "\n\n-- \nWork, as I rewrote it";
    assert_eq!(
        reply::replace_signature(edited, Some("Different"), Some("Home")),
        edited
    );
    assert_eq!(reply::replace_signature("text", None, Some("Home")), "text");
}
#[test]
fn a_signature_cannot_push_a_large_quote_past_the_composer_limit() {
    let line = "x".repeat(70);
    let text = format!("{line}\r\n")
        .repeat((mailstronaut_mail::compose::MAX_BODY_BYTES - 200) / (line.len() + 4));
    let source = mime::project(
        format!(
            "From: Author <author@example.test>\r\nTo: me@example.test\r\nSubject: Topic\r\nMessage-ID: <child@example.test>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}"
        )
        .as_bytes(),
    )
    .expect("MIME");
    reply::prepare(&account(), &[], &source, Kind::Reply).expect("fits without a signature");
    let mut signed = account();
    signed.signature = Some("s".repeat(4096));
    assert!(matches!(
        reply::prepare(&signed, &[], &source, Kind::Reply),
        Err(reply::Error::TooLarge)
    ));
}
