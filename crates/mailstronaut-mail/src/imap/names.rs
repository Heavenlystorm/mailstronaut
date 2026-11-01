//! Readable folder names. The wire name stays the identity; this is for display only.

use std::borrow::Cow;

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};

/// A folder's readable name, decoded from RFC 3501 modified UTF-7. Names come from the
/// server, so anything that is not strictly valid is shown exactly as sent.
#[must_use]
pub fn display_name(wire: &str) -> Cow<'_, str> {
    decode(wire).map_or(Cow::Borrowed(wire), Cow::Owned)
}

fn decode(wire: &str) -> Option<String> {
    let mut decoded = String::with_capacity(wire.len());
    let mut rest = wire;
    while let Some((plain, after)) = rest.split_once('&') {
        decoded.push_str(plain);
        let (shifted, tail) = after.split_once('-')?;
        if shifted.is_empty() {
            decoded.push('&');
        } else {
            decoded.push_str(&decode_shifted(shifted)?);
        }
        rest = tail;
    }
    decoded.push_str(rest);
    Some(decoded)
}

fn decode_shifted(shifted: &str) -> Option<String> {
    if shifted.contains('/') {
        return None;
    }
    let bytes = STANDARD_NO_PAD.decode(shifted.replace(',', "/")).ok()?;
    let (units, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    char::decode_utf16(units.iter().map(|unit| u16::from_be_bytes(*unit)))
        .map(|unit| unit.ok().filter(|character| !misleading(*character)))
        .collect()
}

/// RFC 3501 forbids encoding what could be written plainly, which would let one name
/// imitate another. Control and direction characters could disguise a label the same way.
const fn misleading(character: char) -> bool {
    character.is_ascii()
        || character.is_control()
        || matches!(
            character,
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn valid_names_decode_for_display() {
        for (wire, shown) in [
            ("INBOX", "INBOX"),
            ("Projects/2026 draft", "Projects/2026 draft"),
            ("&AMk-tudes", "Études"),
            ("Sent &- Received", "Sent & Received"),
            ("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-", "Отправленные"),
            ("Projets/&ZeVnLA-", "Projets/日本"),
            ("A&AOk-B&AOk-C", "AéBéC"),
            ("&2D3eAA-", "😀"),
        ] {
            assert_eq!(display_name(wire), shown, "{wire}");
        }
    }

    #[test]
    fn anything_else_from_a_server_is_shown_as_sent_without_panicking() {
        for wire in [
            // Not base64, unterminated, or using the unmodified alphabet.
            "&!!!-",
            "&AMk",
            "&AMk/-",
            // Nonzero trailing bits, an odd byte and a lone surrogate.
            "&AMl-",
            "&AA-",
            "&2D0-",
            // Encoded ASCII could imitate another folder ("a"); newline; right-to-left.
            "&AGE-",
            "&AAo-",
            "&IC4-",
            // One bad section spoils the whole name, not just that section.
            "&AMk-tudes &!!!-",
        ] {
            assert_eq!(display_name(wire), wire);
        }
    }
}
