//! Local priority rules and deadline extraction.
//!
//! This is the rules baseline: no model, no network and no clock. A message's own text and
//! dates decide. Output is versioned so that a later policy or model can replace it, and it
//! carries reasons and evidence so the interface never has to claim more than was found.

pub mod deadline;

use std::sync::LazyLock;

use chrono::{DateTime, TimeZone as _};
use chrono_tz::Tz;
use regex::Regex;
use serde::{Deserialize, Serialize};

pub use deadline::Deadline;

/// Bumped whenever these rules change meaning, so results can be told apart.
pub const POLICY_VERSION: u32 = 3;
/// How far a Date header may differ from server receipt and still be believed.
const MAX_DATE_DISCREPANCY: i64 = 48 * 3600;
const HOUR: i64 = 3600;

/// Why a message got its result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Reason {
    /// The reader is a primary recipient.
    DirectRecipient,
    /// The text asks the reader to do something.
    ActionRequested,
    /// The text says it is urgent.
    Urgent,
    /// A certain deadline was found.
    Deadline,
    /// The text says nothing needs doing.
    NoActionNeeded,
    /// The message looks like a mailing or newsletter.
    BulkMail,
    /// Its deadline had already passed when it arrived.
    AlreadyExpired,
}

/// What analysis could do with this message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Status {
    /// Analyzed by these rules alone.
    RulesOnly,
    /// The text could not be read, such as encrypted mail. Nothing was inferred.
    Unsupported,
}

/// What analysis needs to know about a message.
#[derive(Clone, Copy, Debug)]
pub struct Input<'a> {
    /// Decoded subject.
    pub subject: &'a str,
    /// Readable body, or `None` if there is none to read.
    pub body: Option<&'a str>,
    /// Whether a configured address is a primary recipient.
    pub direct: bool,
    /// The Date header, which the author controls.
    pub authored: Option<i64>,
    /// When the server received it.
    pub received_at: i64,
    /// The reader's zone, for dates that do not state one.
    pub zone: Tz,
    /// Longest priority without a deadline, from the configuration.
    pub max_ttl_hours: u32,
}

/// The versioned result, safe to store and to show.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Analysis {
    /// Rules version.
    pub policy: u32,
    /// What analysis could do.
    pub status: Status,
    /// Whether this belongs in the priority group while unread and unexpired.
    pub priority: bool,
    /// Why, in evidence order.
    pub reasons: Vec<Reason>,
    /// When priority ends, fixed at analysis and never sliding.
    pub expires_at: Option<i64>,
    /// Deadlines in the author's text, including uncertain ones for the reader to see.
    pub deadlines: Vec<Deadline>,
    /// True if the Date header was missing or implausible, so receipt time anchored dates.
    pub anchor_assumed: bool,
}

static NO_ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:no\s+(?:action|response|reply)\s+(?:is\s+)?(?:required|needed|necessary)|for\s+your\s+information\s+only|fyi\s+only|aucune\s+action\s+(?:n'est\s+)?(?:requise|n[ée]cessaire)|rien\s+[àa]\s+faire|pour\s+information\s+seulement|nessuna\s+azione\s+(?:è\s+)?(?:richiesta|necessaria)|non\s+[èe]\s+richiesta\s+alcuna\s+azione|solo\s+per\s+informazione)\b")
        .expect("static pattern")
});
static BULK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:unsubscribe|view\s+(?:this\s+email\s+)?in\s+(?:your\s+)?browser|newsletter|se\s+d[ée]sabonner|d[ée]sinscri\w+|d[ée]sabonn\w+|annulla\s+(?:l'|la\s+)?iscrizione|disiscri\w+|cancella\s+iscrizione)\b")
        .expect("static pattern")
});
static ACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:please\s+(?:reply|respond|confirm|review|complete|submit|send|sign|let\s+me\s+know|answer|fill)|could\s+you|can\s+you|would\s+you|let\s+me\s+know|rsvp|action\s+required|needs?\s+your|waiting\s+for\s+your|merci\s+de\s+(?:r[ée]pondre|confirmer|compl[ée]ter|renvoyer|signer|m'envoyer|nous\s+envoyer|d[ée]poser|me\s+dire)|veuillez\s+(?:r[ée]pondre|confirmer|compl[ée]ter|renvoyer|signer|d[ée]poser|nous\s+envoyer)|pourriez-vous|pouvez-vous|peux-tu|pourrais-tu|action\s+requise|tu\s+peux|vous\s+devez|doit\s+[êe]tre\s+d[ée]pos[ée]|per\s+favore\s+(?:rispondi|conferma|invia|compila|firma|fammi\s+sapere)|si\s+prega\s+di|ti\s+chiedo\s+di|vi\s+chiedo\s+di|puoi|potresti|pu[òo]\s+(?:confermare|inviare|rispondere)|fammi\s+sapere|fatemi\s+sapere|azione\s+richiesta|devi|dovete|[èe]\s+necessario)\b")
        .expect("static pattern")
});
static URGENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:urgent|urgently|asap|as\s+soon\s+as\s+possible|immediately|today\s+if\s+possible|d[èe]s\s+que\s+possible|au\s+plus\s+vite|imm[ée]diatement|urgente|urgentemente|il\s+prima\s+possibile|subito)\b")
        .expect("static pattern")
});

/// The moment a message's relative dates mean: the Date header if it is believable, since
/// mail can arrive late, otherwise receipt. The second value says whether that was assumed.
fn anchor(input: &Input<'_>) -> (DateTime<Tz>, bool) {
    let (seconds, assumed) = match input.authored {
        Some(authored) if (authored - input.received_at).abs() <= MAX_DATE_DISCREPANCY => {
            (authored, false)
        }
        _ => (input.received_at, true),
    };
    let moment = input
        .zone
        .timestamp_opt(seconds, 0)
        .single()
        .unwrap_or_else(|| input.zone.timestamp_opt(0, 0).unwrap());
    (moment, assumed)
}

/// Analyze one message. The same input always gives the same result.
#[must_use]
pub fn analyze(input: &Input<'_>) -> Analysis {
    let Some(body) = input.body else {
        return Analysis {
            policy: POLICY_VERSION,
            status: Status::Unsupported,
            priority: false,
            reasons: Vec::new(),
            expires_at: None,
            deadlines: Vec::new(),
            anchor_assumed: false,
        };
    };
    let authored: String = deadline::authored(body)
        .into_iter()
        .map(|range| &body[range])
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("{}\n{authored}", input.subject);
    let (moment, anchor_assumed) = anchor(input);
    let mut reasons = Vec::new();
    let no_action = NO_ACTION.is_match(&text);
    if no_action {
        reasons.push(Reason::NoActionNeeded);
    }
    // The signature and quoted tails are gone from `authored`, but mailing footers are
    // usually not separated from the text that way.
    let bulk = BULK.is_match(body);
    if bulk {
        reasons.push(Reason::BulkMail);
    }
    let deadlines = if no_action {
        Vec::new()
    } else {
        deadline::constraints(body, moment)
    };
    if input.direct {
        reasons.push(Reason::DirectRecipient);
    }
    let action = ACTION.is_match(&text);
    if action {
        reasons.push(Reason::ActionRequested);
    }
    let urgent = URGENT.is_match(&text);
    if urgent {
        reasons.push(Reason::Urgent);
    }
    // Dates mean authorship, but already-expired requests cannot get a fresh TTL.
    let accepted = deadlines
        .iter()
        .filter_map(|deadline| deadline.ends_at)
        .filter(|ends| *ends > input.received_at)
        .min();
    if accepted.is_some() {
        reasons.push(Reason::Deadline);
    }
    let expired = accepted.is_none()
        && deadlines.iter().any(|deadline| {
            deadline
                .ends_at
                .is_some_and(|ends| ends <= input.received_at)
        });
    if expired {
        reasons.push(Reason::AlreadyExpired);
    }
    let eligible = !no_action && !bulk && !expired;
    let expires_at = if !eligible {
        None
    } else if let Some(ends) = accepted {
        (input.direct || action).then_some(ends)
    } else if action && input.direct {
        let hours: i64 = if urgent { 24 } else { 72 };
        Some(input.received_at + hours.min(i64::from(input.max_ttl_hours)) * HOUR)
    } else {
        None
    };
    Analysis {
        policy: POLICY_VERSION,
        status: Status::RulesOnly,
        priority: expires_at.is_some(),
        reasons,
        expires_at,
        deadlines: deadlines
            .into_iter()
            .filter(|deadline| deadline.date >= moment.date_naive())
            .collect(),
        anchor_assumed,
    }
}

/// Whether a stored result puts this message in the priority group right now. Reading or
/// dismissing it is final: marking it unread again must not bring it back.
#[must_use]
pub const fn in_priority(
    expires_at: Option<i64>,
    now: i64,
    read_cleared: bool,
    dismissed: bool,
) -> bool {
    match expires_at {
        Some(expires_at) => expires_at > now && !read_cleared && !dismissed,
        None => false,
    }
}
