//! The host's signing identity for the runner (spec `ios-device-driver`,
//! "Device discovery and readiness"): an Apple Development identity whose
//! certificate belongs to the team `MOBILE_AGENT_IOS_TEAM` names, valid for
//! code signing in the keychain. The identity is checked on the Mac, with
//! `security`; the device's state on the device.
//!
//! `security find-identity -v -p codesigning` lists the valid identities by
//! their certificate's SHA-1 and name; `security find-certificate -a -Z -p`
//! gives each certificate, whose subject's organizational unit is the team.
//! The name's parenthesized code is the developer's, not the team's.

use std::fmt;
use std::process::Command;

use crate::model::TEAM_VARIABLE;

/// The prefix of an Apple Development identity's name.
pub const DEVELOPMENT_PREFIX: &str = "Apple Development:";

/// Whether this Mac can sign the runner for the team the environment names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdentityCheck {
    /// A valid Apple Development identity of the team.
    Found { name: String, team: String },
    /// `MOBILE_AGENT_IOS_TEAM` is not set.
    NoTeam,
    /// The keychain holds no valid Apple Development identity.
    NoIdentity { team: String },
    /// The keychain's valid Apple Development identities belong to other
    /// teams, those listed (empty when their certificates could not be read).
    OtherTeams { team: String, found: Vec<String> },
    /// `security` could not list the identities; why.
    Unreadable(String),
}

impl IdentityCheck {
    /// Whether the runner can be signed.
    pub fn ok(&self) -> bool {
        matches!(self, IdentityCheck::Found { .. })
    }

    /// Why the runner cannot be signed, for the readiness message; empty
    /// when it can.
    pub fn reason(&self) -> String {
        match self {
            IdentityCheck::Found { .. } => String::new(),
            IdentityCheck::NoTeam => format!("{TEAM_VARIABLE} is not set"),
            IdentityCheck::NoIdentity { team } => format!(
                "the keychain holds no valid Apple Development identity, which the runner of team {team} is signed with"
            ),
            IdentityCheck::OtherTeams { team, found } if found.is_empty() => format!(
                "none of the keychain's valid Apple Development identities belongs to team {team}"
            ),
            IdentityCheck::OtherTeams { team, found } => format!(
                "the keychain's valid Apple Development identities belong to {}, not to team {team}",
                found.join(", ")
            ),
            IdentityCheck::Unreadable(reason) => {
                format!("the keychain's identities could not be listed: {reason}")
            }
        }
    }
}

impl fmt::Display for IdentityCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IdentityCheck::Found { name, .. } => write!(f, "{name}"),
            other => f.write_str(&other.reason()),
        }
    }
}

/// The identity check from the team and the two commands' outputs.
pub fn check(team: Option<&str>, identities: &str, certificates: &str) -> IdentityCheck {
    let Some(team) = team.map(str::trim).filter(|team| !team.is_empty()) else {
        return IdentityCheck::NoTeam;
    };
    let development: Vec<(String, String)> = parse_identities(identities)
        .into_iter()
        .filter(|(_, name)| name.starts_with(DEVELOPMENT_PREFIX))
        .collect();
    if development.is_empty() {
        return IdentityCheck::NoIdentity {
            team: team.to_owned(),
        };
    }
    let certificates = parse_certificates(certificates);
    let mut found: Vec<String> = Vec::new();
    for (sha1, name) in development {
        let unit = certificates
            .iter()
            .find(|(hash, _)| hash.eq_ignore_ascii_case(&sha1))
            .and_then(|(_, der)| subject_unit(der));
        match unit {
            Some(unit) if unit.eq_ignore_ascii_case(team) => {
                return IdentityCheck::Found { name, team: unit };
            }
            Some(unit) if !found.contains(&unit) => found.push(unit),
            _ => {}
        }
    }
    IdentityCheck::OtherTeams {
        team: team.to_owned(),
        found,
    }
}

/// The valid identities `security find-identity -v` lists, as their
/// certificate's SHA-1 in upper-case hex and their name.
pub fn parse_identities(output: &str) -> Vec<(String, String)> {
    output
        .lines()
        .filter_map(|line| {
            let (number, rest) = line.trim().split_once(") ")?;
            number.parse::<u32>().ok()?;
            let (hash, name) = rest.split_once(' ')?;
            if hash.len() != 40 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return None;
            }
            let name = name.trim().strip_prefix('"')?.strip_suffix('"')?;
            Some((hash.to_ascii_uppercase(), name.to_owned()))
        })
        .collect()
}

/// The certificates `security find-certificate -a -Z -p` prints, as their
/// SHA-1 in upper-case hex and their DER bytes.
pub fn parse_certificates(output: &str) -> Vec<(String, Vec<u8>)> {
    let mut certificates = Vec::new();
    let mut hash: Option<String> = None;
    let mut pem: Option<String> = None;
    for line in output.lines().map(str::trim) {
        if let Some(sha1) = line.strip_prefix("SHA-1 hash:") {
            hash = Some(sha1.trim().to_ascii_uppercase());
        } else if line == "-----BEGIN CERTIFICATE-----" {
            pem = Some(String::new());
        } else if line == "-----END CERTIFICATE-----" {
            if let (Some(sha1), Some(body)) = (hash.take(), pem.take())
                && let Some(der) = decode_base64(&body)
            {
                certificates.push((sha1, der));
            }
        } else if let Some(body) = pem.as_mut() {
            body.push_str(line);
        }
    }
    certificates
}

const SEQUENCE: u8 = 0x30;
const SET: u8 = 0x31;
const OBJECT_IDENTIFIER: u8 = 0x06;
const EXPLICIT_VERSION: u8 = 0xA0;
/// The object identifier 2.5.4.11, organizationalUnitName.
const ORGANIZATIONAL_UNIT: &[u8] = &[0x55, 0x04, 0x0B];

/// One DER element at the front of `input`: its tag, its content and what
/// follows it.
fn element(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, mut rest) = rest.split_first()?;
    let length = if first < 0x80 {
        usize::from(first)
    } else {
        let count = usize::from(first & 0x7F);
        if count == 0 || count > 4 || rest.len() < count {
            return None;
        }
        let (bytes, after) = rest.split_at(count);
        rest = after;
        bytes
            .iter()
            .fold(0usize, |length, &byte| (length << 8) | usize::from(byte))
    };
    if rest.len() < length {
        return None;
    }
    let (content, after) = rest.split_at(length);
    Some((tag, content, after))
}

/// The organizational unit of a DER certificate's subject: the team of an
/// Apple Development certificate. The issuer's units, such as Apple's "G3",
/// are not it.
pub fn subject_unit(der: &[u8]) -> Option<String> {
    let (SEQUENCE, certificate, _) = element(der)? else {
        return None;
    };
    let (SEQUENCE, tbs, _) = element(certificate)? else {
        return None;
    };
    // TBSCertificate: [0] version, serialNumber, signature, issuer,
    // validity, subject, and more after.
    let mut rest = tbs;
    let (tag, _, after) = element(rest)?;
    if tag == EXPLICIT_VERSION {
        rest = after;
    }
    for _ in 0..4 {
        rest = element(rest)?.2;
    }
    let (SEQUENCE, subject, _) = element(rest)? else {
        return None;
    };
    let mut sets = subject;
    while !sets.is_empty() {
        let (tag, set, after) = element(sets)?;
        sets = after;
        if tag != SET {
            continue;
        }
        let mut attributes = set;
        while !attributes.is_empty() {
            let (tag, attribute, after) = element(attributes)?;
            attributes = after;
            if tag != SEQUENCE {
                continue;
            }
            let (OBJECT_IDENTIFIER, oid, value) = element(attribute)? else {
                continue;
            };
            if oid == ORGANIZATIONAL_UNIT {
                let (_, text, _) = element(value)?;
                return String::from_utf8(text.to_vec()).ok();
            }
        }
    }
    None
}

/// Standard base64 with padding, whitespace ignored; `None` when it is not.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn sextet(symbol: u8) -> Option<u32> {
        let value = match symbol {
            b'A'..=b'Z' => symbol - b'A',
            b'a'..=b'z' => symbol - b'a' + 26,
            b'0'..=b'9' => symbol - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        Some(u32::from(value))
    }
    let symbols: Vec<u8> = text
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect();
    if !symbols.len().is_multiple_of(4) {
        return None;
    }
    let blocks = symbols.len() / 4;
    let mut bytes = Vec::with_capacity(blocks * 3);
    for (index, block) in symbols.chunks(4).enumerate() {
        let padding = block
            .iter()
            .rev()
            .take_while(|&&symbol| symbol == b'=')
            .count();
        if padding > 2 || (padding > 0 && index + 1 != blocks) {
            return None;
        }
        let mut bits = 0u32;
        for (position, &symbol) in block.iter().enumerate() {
            let value = if position >= 4 - padding {
                0
            } else {
                sextet(symbol)?
            };
            bits = (bits << 6) | value;
        }
        let decoded = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        bytes.extend_from_slice(&decoded[..3 - padding]);
    }
    Some(bytes)
}

/// The host's signing identity for the team in `MOBILE_AGENT_IOS_TEAM`,
/// through `security` (about 40 ms per command on the spike host).
pub fn host_identity() -> IdentityCheck {
    let team = std::env::var(TEAM_VARIABLE).ok();
    let team = team
        .as_deref()
        .map(str::trim)
        .filter(|team| !team.is_empty());
    let Some(team) = team else {
        return IdentityCheck::NoTeam;
    };
    let run = |args: &[&str]| -> Result<String, String> {
        let output = Command::new("security")
            .args(args)
            .output()
            .map_err(|error| format!("security {}: {error}", args.join(" ")))?;
        if !output.status.success() {
            return Err(format!(
                "security {} exited with {}: {}",
                args.join(" "),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let identities = match run(&["find-identity", "-v", "-p", "codesigning"]) {
        Ok(text) => text,
        Err(reason) => return IdentityCheck::Unreadable(reason),
    };
    // A keychain without the certificate makes find-certificate exit 44;
    // the identities' check then names no team.
    let certificates = run(&[
        "find-certificate",
        "-a",
        "-Z",
        "-p",
        "-c",
        "Apple Development",
    ])
    .unwrap_or_default();
    check(Some(team), &identities, &certificates)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `security find-identity -v -p codesigning` as this host printed it,
    /// the hash and name replaced.
    const IDENTITIES: &str = "  1) 0123456789ABCDEF0123456789ABCDEF01234567 \"Apple Development: placeholder-developer (PLACEHOLD0)\"\n  2) 89ABCDEF0123456789ABCDEF0123456789ABCDEF \"Developer ID Application: placeholder-company (PLACEHOLD2)\"\n     2 valid identities found\n";

    fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        let length = content.len();
        if length < 0x80 {
            out.push(length as u8);
        } else if length < 0x100 {
            out.extend([0x81, length as u8]);
        } else {
            out.extend([0x82, (length >> 8) as u8, length as u8]);
        }
        out.extend_from_slice(content);
        out
    }

    fn name(units: &[(&[u8], &str)]) -> Vec<u8> {
        let mut sets = Vec::new();
        for (oid, value) in units {
            let attribute = [tlv(0x06, oid), tlv(0x0C, value.as_bytes())].concat();
            sets.extend(tlv(0x31, &tlv(0x30, &attribute)));
        }
        tlv(0x30, &sets)
    }

    const CN: &[u8] = &[0x55, 0x04, 0x03];
    const OU: &[u8] = &[0x55, 0x04, 0x0B];

    /// A certificate with the shape of an Apple Development one: the issuer
    /// has its own unit, the subject a long name and the team as its unit.
    fn certificate(team: &str) -> Vec<u8> {
        let version = tlv(0xA0, &tlv(0x02, &[2]));
        let serial = tlv(0x02, &[0x01, 0x23, 0x45]);
        let algorithm = tlv(
            0x30,
            &tlv(
                0x06,
                &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B],
            ),
        );
        let issuer = name(&[
            (
                CN,
                "Apple Worldwide Developer Relations Certification Authority",
            ),
            (OU, "G3"),
        ]);
        let validity = tlv(
            0x30,
            &[tlv(0x17, b"261001000000Z"), tlv(0x17, b"271001000000Z")].concat(),
        );
        let long_name = format!(
            "Apple Development: placeholder-developer (PLACEHOLD0){}",
            " ".repeat(140)
        );
        let subject = name(&[(CN, &long_name), (OU, team)]);
        let tbs = tlv(
            0x30,
            &[
                version,
                serial,
                algorithm.clone(),
                issuer,
                validity,
                subject,
            ]
            .concat(),
        );
        tlv(0x30, &[tbs, algorithm, tlv(0x03, &[0x00, 0xFF])].concat())
    }

    fn encode_base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let block = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let bits =
                (u32::from(block[0]) << 16) | (u32::from(block[1]) << 8) | u32::from(block[2]);
            for index in 0..4 {
                if index <= chunk.len() {
                    out.push(ALPHABET[((bits >> (18 - 6 * index)) & 0x3F) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    fn pem_listing(certificates: &[(&str, Vec<u8>)]) -> String {
        let mut out = String::new();
        for (sha1, der) in certificates {
            out.push_str(
                "SHA-256 hash: 00000000000000000000000000000000000000000000000000000000000000AA\n",
            );
            out.push_str(&format!(
                "SHA-1 hash: {sha1}\n-----BEGIN CERTIFICATE-----\n"
            ));
            let text = encode_base64(der);
            for line in text.as_bytes().chunks(64) {
                out.push_str(std::str::from_utf8(line).expect("ascii"));
                out.push('\n');
            }
            out.push_str("-----END CERTIFICATE-----\n");
        }
        out
    }

    #[test]
    fn valid_identities_are_listed_with_their_hash_and_name() {
        assert_eq!(
            parse_identities(IDENTITIES),
            vec![
                (
                    "0123456789ABCDEF0123456789ABCDEF01234567".to_owned(),
                    "Apple Development: placeholder-developer (PLACEHOLD0)".to_owned()
                ),
                (
                    "89ABCDEF0123456789ABCDEF0123456789ABCDEF".to_owned(),
                    "Developer ID Application: placeholder-company (PLACEHOLD2)".to_owned()
                ),
            ]
        );
        assert!(parse_identities("     0 valid identities found\n").is_empty());
    }

    #[test]
    fn base64_decodes_with_padding_and_line_breaks_and_refuses_anything_else() {
        assert_eq!(decode_base64("").as_deref(), Some(&b""[..]));
        assert_eq!(decode_base64("Zg==").as_deref(), Some(&b"f"[..]));
        assert_eq!(decode_base64("Zm8=").as_deref(), Some(&b"fo"[..]));
        assert_eq!(decode_base64("Zm9v").as_deref(), Some(&b"foo"[..]));
        assert_eq!(
            decode_base64("Zm9v\nYmFy\n").as_deref(),
            Some(&b"foobar"[..])
        );
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_base64(&encode_base64(&bytes)), Some(bytes));
        assert_eq!(decode_base64("Zm9v!"), None);
        assert_eq!(decode_base64("Zm9"), None);
    }

    #[test]
    fn the_team_is_the_subjects_unit_not_the_issuers() {
        assert_eq!(
            subject_unit(&certificate("TEAMPLACE1")).as_deref(),
            Some("TEAMPLACE1")
        );
        assert_eq!(subject_unit(&[0x30, 0x03, 0x02, 0x01]), None, "truncated");
        assert_eq!(subject_unit(b"not a certificate"), None);
    }

    #[test]
    fn certificates_are_read_with_their_hash() {
        let der = certificate("TEAMPLACE1");
        let listing = pem_listing(&[("0123456789ABCDEF0123456789ABCDEF01234567", der.clone())]);
        assert_eq!(
            parse_certificates(&listing),
            vec![("0123456789ABCDEF0123456789ABCDEF01234567".to_owned(), der)]
        );
        assert!(parse_certificates("").is_empty());
    }

    #[test]
    fn the_identity_of_the_named_team_is_found() {
        let certificates = pem_listing(&[
            (
                "0123456789ABCDEF0123456789ABCDEF01234567",
                certificate("TEAMPLACE1"),
            ),
            (
                "89ABCDEF0123456789ABCDEF0123456789ABCDEF",
                certificate("TEAMPLACE2"),
            ),
        ]);
        let found = check(Some("TEAMPLACE1"), IDENTITIES, &certificates);
        assert_eq!(
            found,
            IdentityCheck::Found {
                name: "Apple Development: placeholder-developer (PLACEHOLD0)".into(),
                team: "TEAMPLACE1".into(),
            }
        );
        assert!(found.ok());
        assert_eq!(found.reason(), "");
    }

    #[test]
    fn a_missing_team_identity_or_team_says_which() {
        let certificates = pem_listing(&[(
            "0123456789ABCDEF0123456789ABCDEF01234567",
            certificate("TEAMPLACE1"),
        )]);
        let no_team = check(None, IDENTITIES, &certificates);
        assert_eq!(no_team, IdentityCheck::NoTeam);
        assert!(!no_team.ok());
        assert!(
            no_team.reason().contains("MOBILE_AGENT_IOS_TEAM"),
            "{}",
            no_team.reason()
        );

        let none = check(Some("TEAMPLACE1"), "     0 valid identities found\n", "");
        assert_eq!(
            none,
            IdentityCheck::NoIdentity {
                team: "TEAMPLACE1".into()
            }
        );
        assert!(
            none.reason()
                .contains("no valid Apple Development identity"),
            "{}",
            none.reason()
        );

        let other = check(Some("TEAMPLACE9"), IDENTITIES, &certificates);
        assert_eq!(
            other,
            IdentityCheck::OtherTeams {
                team: "TEAMPLACE9".into(),
                found: vec!["TEAMPLACE1".into()],
            }
        );
        assert!(other.reason().contains("TEAMPLACE1"), "{}", other.reason());

        // A Developer ID identity of the team does not sign for a device.
        let developer_id_only = "  1) 89ABCDEF0123456789ABCDEF0123456789ABCDEF \"Developer ID Application: placeholder-company (PLACEHOLD2)\"\n     1 valid identities found\n";
        assert_eq!(
            check(Some("TEAMPLACE1"), developer_id_only, &certificates),
            IdentityCheck::NoIdentity {
                team: "TEAMPLACE1".into()
            }
        );
        assert!(!IdentityCheck::Unreadable("security failed".into()).ok());
    }
}
