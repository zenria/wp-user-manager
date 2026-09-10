//! Parsing of the alias file: a pure e-mail mapping, kept apart from the
//! reference list.
//!
//! Format: one group per line, at least two addresses (separated by
//! whitespace, `,` or `;`) that all designate the same person. Blank lines and
//! `#` comments are ignored.
//!
//! A group is a *set*: the order of the addresses on a line carries no
//! meaning, and no address is privileged. Being listed here never makes an
//! address a reference entry — only `reference.rs` decides who must exist.
//! Groups sharing an address are merged, so the mapping does not depend on the
//! order of the lines either.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::reference::{Email, Normalizer};

#[derive(Debug, Error)]
pub enum AliasError {
    #[error("cannot read alias file `{path}`")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("line {line}: `{value}` is not a valid e-mail address")]
    InvalidEmail { line: usize, value: String },
    #[error(
        "line {line}: an alias group needs at least two addresses (found `{value}`); \
         the alias file only maps addresses to one another, it does not list users"
    )]
    LonelyEmail { line: usize, value: String },
}

/// A set of addresses designating the same person.
#[derive(Debug, Clone)]
pub struct AliasGroup {
    /// Smallest normalized key of the group; identifies the group without
    /// privileging any address.
    pub canonical: String,
    /// The addresses as written, ordered by normalized key for a stable
    /// display.
    pub emails: Vec<Email>,
    /// Alias file lines this group was built from.
    pub lines: Vec<usize>,
}

impl AliasGroup {
    pub fn display(&self) -> String {
        self.emails
            .iter()
            .map(Email::raw)
            .collect::<Vec<_>>()
            .join(" = ")
    }
}

/// The mapping itself: from any known address to the group it belongs to.
#[derive(Debug, Clone, Default)]
pub struct AliasMap {
    path: Option<PathBuf>,
    /// normalized key -> index in `groups`
    class_of: HashMap<String, usize>,
    groups: Vec<AliasGroup>,
}

impl AliasMap {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn load(path: &Path, normalizer: &Normalizer) -> Result<Self, AliasError> {
        let content = std::fs::read_to_string(path).map_err(|source| AliasError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut map = Self::parse(&content, normalizer)?;
        map.path = Some(path.to_path_buf());
        Ok(map)
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn groups(&self) -> &[AliasGroup] {
        &self.groups
    }

    /// The key identifying the person behind this address: the canonical key
    /// of its alias group, or the address itself when it has no alias.
    pub fn class_of<'a>(&'a self, key: &'a str) -> &'a str {
        match self.class_of.get(key) {
            Some(&index) => &self.groups[index].canonical,
            None => key,
        }
    }

    pub fn group_of(&self, key: &str) -> Option<&AliasGroup> {
        self.class_of.get(key).map(|&index| &self.groups[index])
    }

    pub fn parse(content: &str, normalizer: &Normalizer) -> Result<Self, AliasError> {
        // Groups are built incrementally; a line touching several existing
        // groups merges them, which makes the result independent of the order
        // of the lines.
        let mut builders: Vec<Option<Builder>> = Vec::new();
        let mut class_of: HashMap<String, usize> = HashMap::new();

        for (index, raw_line) in content.lines().enumerate() {
            let line = index + 1;
            let text = raw_line.split('#').next().unwrap_or("").trim();
            if text.is_empty() {
                continue;
            }

            let mut emails = BTreeMap::new();
            for token in text.split([',', ';', ' ', '\t']).filter(|t| !t.is_empty()) {
                let email =
                    Email::new(token, normalizer).ok_or_else(|| AliasError::InvalidEmail {
                        line,
                        value: token.to_string(),
                    })?;
                emails.insert(email.key().to_string(), email);
            }
            if emails.len() < 2 {
                let value = emails
                    .values()
                    .next()
                    .map(|email| email.raw().to_string())
                    .unwrap_or_default();
                return Err(AliasError::LonelyEmail { line, value });
            }

            let mut target = Builder {
                emails,
                lines: vec![line],
            };
            let mut touched: Vec<usize> = target
                .emails
                .keys()
                .filter_map(|key| class_of.get(key).copied())
                .collect();
            touched.sort_unstable();
            touched.dedup();

            for &existing in &touched {
                if let Some(other) = builders[existing].take() {
                    target.absorb(other);
                }
            }

            let slot = touched.first().copied().unwrap_or(builders.len());
            for key in target.emails.keys() {
                class_of.insert(key.clone(), slot);
            }
            if slot == builders.len() {
                builders.push(Some(target));
            } else {
                builders[slot] = Some(target);
            }
        }

        // Compact: drop the slots emptied by merges and renumber.
        let mut groups = Vec::new();
        let mut final_class_of = HashMap::new();
        for builder in builders.into_iter().flatten() {
            let group = builder.finish();
            let index = groups.len();
            for email in &group.emails {
                final_class_of.insert(email.key().to_string(), index);
            }
            groups.push(group);
        }

        Ok(Self {
            path: None,
            class_of: final_class_of,
            groups,
        })
    }
}

struct Builder {
    emails: BTreeMap<String, Email>,
    lines: Vec<usize>,
}

impl Builder {
    fn absorb(&mut self, other: Builder) {
        self.emails.extend(other.emails);
        self.lines.extend(other.lines);
    }

    fn finish(self) -> AliasGroup {
        let mut lines = self.lines;
        lines.sort_unstable();
        lines.dedup();
        let canonical = self
            .emails
            .keys()
            .next()
            .cloned()
            .expect("an alias group always holds at least two addresses");
        AliasGroup {
            canonical,
            emails: self.emails.into_values().collect(),
            lines,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(content: &str) -> AliasMap {
        AliasMap::parse(content, &Normalizer::default()).unwrap()
    }

    #[test]
    fn maps_every_address_of_a_group_to_the_same_class() {
        let map = parse("a@example.com, a@old.com\n");
        assert_eq!(map.groups().len(), 1);
        assert_eq!(map.class_of("a@example.com"), map.class_of("a@old.com"));
    }

    #[test]
    fn leaves_unknown_addresses_untouched() {
        let map = parse("a@example.com a@old.com\n");
        assert_eq!(map.class_of("b@example.com"), "b@example.com");
        assert!(map.group_of("b@example.com").is_none());
    }

    #[test]
    fn does_not_depend_on_the_order_inside_a_line() {
        let one = parse("a@example.com, z@old.com\n");
        let other = parse("z@old.com, a@example.com\n");
        assert_eq!(
            one.class_of("z@old.com"),
            other.class_of("a@example.com"),
            "the canonical key must not depend on the position on the line"
        );
        assert_eq!(one.groups()[0].emails.len(), other.groups()[0].emails.len());
    }

    #[test]
    fn merges_groups_sharing_an_address_whatever_the_line_order() {
        let one = parse("a@example.com b@old.com\nb@old.com c@older.com\n");
        let other = parse("b@old.com c@older.com\na@example.com b@old.com\n");
        for map in [&one, &other] {
            assert_eq!(map.groups().len(), 1);
            assert_eq!(map.groups()[0].emails.len(), 3);
            assert_eq!(map.class_of("a@example.com"), map.class_of("c@older.com"));
        }
        assert_eq!(one.class_of("c@older.com"), other.class_of("a@example.com"));
    }

    #[test]
    fn keeps_separate_groups_separate() {
        let map = parse("a@example.com a@old.com\nb@example.com b@old.com\n");
        assert_eq!(map.groups().len(), 2);
        assert_ne!(map.class_of("a@old.com"), map.class_of("b@old.com"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let map = parse("# aliases\n\n  a@example.com a@old.com # renamed\n");
        assert_eq!(map.groups().len(), 1);
        assert_eq!(map.groups()[0].lines, vec![3]);
    }

    #[test]
    fn rejects_a_line_holding_a_single_address() {
        let err = AliasMap::parse("a@example.com\n", &Normalizer::default()).unwrap_err();
        assert!(matches!(err, AliasError::LonelyEmail { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_addresses() {
        let err = AliasMap::parse("a@example.com oops\n", &Normalizer::default()).unwrap_err();
        assert!(matches!(err, AliasError::InvalidEmail { line: 1, .. }));
    }
}
