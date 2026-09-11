//! Parsing of the reference file: the source of truth for which users must
//! exist in WordPress.
//!
//! Format: one address per line — one line, one user. Blank lines and `#`
//! comments are ignored. Alternative addresses of the same person do *not*
//! belong here: they live in the alias file (see `aliases.rs`), which only
//! maps addresses to one another.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::aliases::AliasMap;

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
    #[error(
        "line {line}: several addresses on one line (`{value}`); the reference file takes \
         one address per line, declare alternative addresses in the alias file"
    )]
    SeveralEmails { line: usize, value: String },
    #[error("line {line}: e-mail `{value}` is already listed on line {first_line}")]
    DuplicateEmail {
        line: usize,
        first_line: usize,
        value: String,
    },
    #[error(
        "line {line}: `{value}` is an alias of `{other}` (line {first_line}); the same person \
         must be listed only once"
    )]
    AliasOfAnotherEntry {
        line: usize,
        first_line: usize,
        value: String,
        other: String,
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

/// One line of the reference file: one person who must exist in WordPress.
#[derive(Debug, Clone)]
pub struct Identity {
    /// 1-based line number in the reference file; 0 when the identity does not
    /// come from a file (see `Identity::ad_hoc`).
    pub line: usize,
    /// The address as listed; the one used if the account has to be created.
    pub email: Email,
    /// Key shared with every alias of this address (see `AliasMap`).
    pub class: String,
}

impl Identity {
    /// An identity given on the command line rather than read from the
    /// reference file. It is resolved through the alias map like any other,
    /// so it matches an account registered under one of its aliases.
    pub fn ad_hoc(email: Email, aliases: &AliasMap) -> Self {
        let class = aliases.class_of(email.key()).to_string();
        Self {
            line: 0,
            email,
            class,
        }
    }

    pub fn email(&self) -> &Email {
        &self.email
    }

    /// The address, plus its known aliases, for display.
    pub fn display(&self, aliases: &AliasMap) -> String {
        match aliases.group_of(self.email.key()) {
            None => self.email.raw().to_string(),
            Some(group) => {
                let others: Vec<&str> = group
                    .emails
                    .iter()
                    .filter(|email| email.key() != self.email.key())
                    .map(Email::raw)
                    .collect();
                if others.is_empty() {
                    self.email.raw().to_string()
                } else {
                    format!("{}  (aliases: {})", self.email.raw(), others.join(", "))
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Reference {
    pub path: PathBuf,
    pub identities: Vec<Identity>,
}

impl Reference {
    pub fn load(
        path: &Path,
        normalizer: &Normalizer,
        aliases: &AliasMap,
    ) -> Result<Self, ReferenceError> {
        let content = std::fs::read_to_string(path).map_err(|source| ReferenceError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let identities = parse(&content, normalizer, aliases)?;
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
}

pub fn parse(
    content: &str,
    normalizer: &Normalizer,
    aliases: &AliasMap,
) -> Result<Vec<Identity>, ReferenceError> {
    let mut identities: Vec<Identity> = Vec::new();
    // class key -> index in `identities`, to catch a person listed twice
    // (directly, or through one of their aliases).
    let mut seen: HashMap<String, usize> = HashMap::new();

    for (index, raw_line) in content.lines().enumerate() {
        let line = index + 1;
        let text = raw_line.split('#').next().unwrap_or("").trim();
        if text.is_empty() {
            continue;
        }

        let mut tokens = text.split([',', ';', ' ', '\t']).filter(|t| !t.is_empty());
        let token = match tokens.next() {
            Some(token) => token,
            None => continue,
        };
        if tokens.next().is_some() {
            return Err(ReferenceError::SeveralEmails {
                line,
                value: text.to_string(),
            });
        }

        let email = Email::new(token, normalizer).ok_or_else(|| ReferenceError::InvalidEmail {
            line,
            value: token.to_string(),
        })?;
        let class = aliases.class_of(email.key()).to_string();

        if let Some(&first) = seen.get(&class) {
            let previous = &identities[first];
            return Err(if previous.email.key() == email.key() {
                ReferenceError::DuplicateEmail {
                    line,
                    first_line: previous.line,
                    value: email.raw().to_string(),
                }
            } else {
                ReferenceError::AliasOfAnotherEntry {
                    line,
                    first_line: previous.line,
                    value: email.raw().to_string(),
                    other: previous.email.raw().to_string(),
                }
            });
        }

        seen.insert(class.clone(), identities.len());
        identities.push(Identity { line, email, class });
    }

    Ok(identities)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_aliases() -> AliasMap {
        AliasMap::empty()
    }

    fn alias_map(content: &str) -> AliasMap {
        AliasMap::parse(content, &Normalizer::default()).unwrap()
    }

    fn parse_with(content: &str, aliases: &AliasMap) -> Result<Vec<Identity>, ReferenceError> {
        parse(content, &Normalizer::default(), aliases)
    }

    #[test]
    fn reads_one_identity_per_line() {
        let identities = parse_with("a@example.com\nb@example.com\n", &no_aliases()).unwrap();
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].email().raw(), "a@example.com");
        assert_eq!(identities[1].line, 2);
    }

    #[test]
    fn ignores_blank_lines_and_comments() {
        let identities =
            parse_with("# header\n\n  a@example.com  # the boss\n", &no_aliases()).unwrap();
        assert_eq!(identities.len(), 1);
        assert_eq!(identities[0].line, 3);
    }

    #[test]
    fn compares_addresses_case_insensitively() {
        let identities = parse_with("Alice@Example.COM\n", &no_aliases()).unwrap();
        assert_eq!(identities[0].email().raw(), "Alice@Example.COM");
        assert_eq!(identities[0].class, "alice@example.com");
    }

    #[test]
    fn strips_plus_tags_when_asked() {
        let normalizer = Normalizer {
            strip_plus_tags: true,
        };
        let identities = parse("alice+wp@example.com\n", &normalizer, &no_aliases()).unwrap();
        assert_eq!(identities[0].class, "alice@example.com");
    }

    #[test]
    fn an_ad_hoc_identity_resolves_through_the_alias_map() {
        let aliases = alias_map("new@example.com old@legacy.com\n");
        let normalizer = Normalizer::default();
        let email = Email::new("Old@Legacy.com", &normalizer).unwrap();

        let identity = Identity::ad_hoc(email, &aliases);

        assert_eq!(identity.line, 0);
        // The account is created with the address as typed...
        assert_eq!(identity.email().raw(), "Old@Legacy.com");
        // ...but the person is identified by the group's canonical key.
        assert_eq!(identity.class, "new@example.com");
    }

    #[test]
    fn shares_the_class_of_its_alias_group() {
        let aliases = alias_map("a@example.com a@old.com\n");
        let identities = parse_with("a@example.com\n", &aliases).unwrap();
        assert_eq!(identities[0].class, aliases.class_of("a@old.com"));
        assert_eq!(
            identities[0].display(&aliases),
            "a@example.com  (aliases: a@old.com)"
        );
    }

    #[test]
    fn an_alias_alone_is_not_a_reference_entry() {
        let aliases = alias_map("a@example.com a@old.com\n");
        let identities = parse_with("b@example.com\n", &aliases).unwrap();
        assert_eq!(identities.len(), 1, "the alias file must not add entries");
        assert_eq!(identities[0].email().raw(), "b@example.com");
    }

    #[test]
    fn rejects_several_addresses_on_one_line() {
        let err = parse_with("a@example.com, a@old.com\n", &no_aliases()).unwrap_err();
        assert!(matches!(err, ReferenceError::SeveralEmails { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_addresses() {
        let err = parse_with("not-an-email\n", &no_aliases()).unwrap_err();
        assert!(matches!(err, ReferenceError::InvalidEmail { line: 1, .. }));
    }

    #[test]
    fn rejects_the_same_address_twice() {
        let err = parse_with("a@example.com\na@example.com\n", &no_aliases()).unwrap_err();
        assert!(matches!(
            err,
            ReferenceError::DuplicateEmail {
                line: 2,
                first_line: 1,
                ..
            }
        ));
    }

    #[test]
    fn rejects_two_entries_that_are_aliases_of_each_other() {
        let aliases = alias_map("a@example.com a@old.com\n");
        let err = parse_with("a@example.com\na@old.com\n", &aliases).unwrap_err();
        assert!(matches!(
            err,
            ReferenceError::AliasOfAnotherEntry {
                line: 2,
                first_line: 1,
                ..
            }
        ));
    }
}
