//! Parsing of the reference file: the source of truth for which users should
//! exist in WordPress.
//!
//! Format: one *identity* per line. A line may hold several e-mail addresses
//! (separated by whitespace, `,` or `;`) that all designate the same person.
//! The first address of a line is the primary one, used when the user has to
//! be created. Blank lines and `#` comments are ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum ReferenceError {
    #[error("cannot read reference file `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: `{value}` is not a valid e-mail address")]
    InvalidEmail { line: usize, value: String },
    #[error("line {line}: e-mail `{value}` is already listed on line {first_line}")]
    DuplicateEmail {
        line: usize,
        first_line: usize,
        value: String,
    },
    #[error("reference file `{path}` does not contain any e-mail address")]
    Empty { path: PathBuf },
}

/// How raw e-mail addresses are turned into comparison keys.
#[derive(Debug, Clone, Copy, Default)]
pub struct Normalizer {
    /// Drop `+tag` suffixes in the local part (`a+wp@x.com` == `a@x.com`).
    pub strip_plus_tags: bool,
}

impl Normalizer {
    pub fn normalize(&self, raw: &str) -> String {
        let trimmed = raw.trim().to_lowercase();
        let Some((local, domain)) = trimmed.split_once('@') else {
            return trimmed;
        };
        let local = if self.strip_plus_tags {
            local.split_once('+').map_or(local, |(head, _)| head)
        } else {
            local
        };
        format!("{local}@{domain}")
    }
}

/// An e-mail address, together with the key used to compare it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Email {
    raw: String,
    key: String,
}

impl Email {
    pub fn new(raw: &str, normalizer: &Normalizer) -> Option<Self> {
        let raw = raw.trim().to_string();
        if !is_plausible_email(&raw) {
            return None;
        }
        let key = normalizer.normalize(&raw);
        Some(Self { raw, key })
    }

    /// The address as written, to be displayed and sent to WordPress.
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// The normalized comparison key.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Local part of the address as written.
    pub fn local_part(&self) -> &str {
        self.raw.split('@').next().unwrap_or(&self.raw)
    }
}

fn is_plausible_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !value.contains(char::is_whitespace)
        && value.matches('@').count() == 1
}

/// One person, possibly known under several e-mail addresses.
#[derive(Debug, Clone)]
pub struct Identity {
    /// 1-based line number in the reference file.
    pub line: usize,
    /// Non-empty; the first entry is the primary address.
    pub emails: Vec<Email>,
}

impl Identity {
    pub fn primary(&self) -> &Email {
        &self.emails[0]
    }

    pub fn aliases(&self) -> &[Email] {
        &self.emails[1..]
    }

    pub fn matches_key(&self, key: &str) -> bool {
        self.emails.iter().any(|email| email.key() == key)
    }

    /// All addresses as written, for display.
    pub fn display(&self) -> String {
        self.emails
            .iter()
            .map(Email::raw)
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub path: PathBuf,
    pub identities: Vec<Identity>,
}

impl Reference {
    pub fn load(path: &Path, normalizer: &Normalizer) -> Result<Self, ReferenceError> {
        let content = std::fs::read_to_string(path).map_err(|source| ReferenceError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let identities = parse(&content, normalizer)?;
        if identities.is_empty() {
            return Err(ReferenceError::Empty {
                path: path.to_path_buf(),
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
            identities,
        })
    }

    pub fn email_count(&self) -> usize {
        self.identities.iter().map(|i| i.emails.len()).sum()
    }
}

pub fn parse(content: &str, normalizer: &Normalizer) -> Result<Vec<Identity>, ReferenceError> {
    let mut identities = Vec::new();
    // normalized key -> line where it was first seen
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (index, raw_line) in content.lines().enumerate() {
        let line = index + 1;
        let text = raw_line.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }

        let mut emails = Vec::new();
        for token in text.split([',', ';', ' ', '\t']).filter(|t| !t.is_empty()) {
            let email =
                Email::new(token, normalizer).ok_or_else(|| ReferenceError::InvalidEmail {
                    line,
                    value: token.to_string(),
                })?;
            if let Some(&first_line) = seen.get(email.key()) {
                return Err(ReferenceError::DuplicateEmail {
                    line,
                    first_line,
                    value: email.raw().to_string(),
                });
            }
            seen.insert(email.key().to_string(), line);
            emails.push(email);
        }

        if !emails.is_empty() {
            identities.push(Identity { line, emails });
        }
    }

    Ok(identities)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Normalizer {
        Normalizer::default()
    }

    #[test]
    fn parses_one_identity_per_line() {
        let identities = parse("a@example.com\nb@example.com\n", &plain()).unwrap();
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].primary().raw(), "a@example.com");
        assert_eq!(identities[1].line, 2);
    }

    #[test]
    fn groups_several_addresses_of_the_same_person() {
        let identities = parse("a@example.com, a@other.com; a2@example.com\n", &plain()).unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].emails.len(), 3);
        assert_eq!(identities[0].primary().raw(), "a@example.com");
        assert_eq!(identities[0].aliases().len(), 2);
        assert!(identities[0].matches_key("a2@example.com"));
    }

    #[test]
    fn ignores_blank_lines_and_comments() {
        let content = "# header\n\n  a@example.com  # the boss\n\n";
        let identities = parse(content, &plain()).unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].line, 3);
    }

    #[test]
    fn compares_addresses_case_insensitively() {
        let identities = parse("Alice@Example.COM\n", &plain()).unwrap();
        assert_eq!(identities[0].primary().raw(), "Alice@Example.COM");
        assert!(identities[0].matches_key("alice@example.com"));
    }

    #[test]
    fn strips_plus_tags_when_asked() {
        let normalizer = Normalizer {
            strip_plus_tags: true,
        };
        let identities = parse("alice+wp@example.com\n", &normalizer).unwrap();
        assert!(identities[0].matches_key("alice@example.com"));
    }

    #[test]
    fn rejects_invalid_addresses() {
        let err = parse("not-an-email\n", &plain()).unwrap_err();
        assert!(matches!(err, ReferenceError::InvalidEmail { line: 1, .. }));
    }

    #[test]
    fn rejects_the_same_address_on_two_lines() {
        let err = parse("a@example.com\nb@example.com a@example.com\n", &plain()).unwrap_err();
        assert!(matches!(
            err,
            ReferenceError::DuplicateEmail {
                line: 2,
                first_line: 1,
                ..
            }
        ));
    }
}
