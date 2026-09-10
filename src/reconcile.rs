//! Matching of the reference file against the WordPress user base.

use std::collections::HashMap;

use crate::reference::{Identity, Normalizer, Reference};
use crate::wp::WpUser;

/// A reference identity and the WordPress user(s) whose e-mail matches one of
/// its addresses.
#[derive(Debug, Clone)]
pub struct IdentityMatch {
    pub identity: Identity,
    pub users: Vec<WpUser>,
}

impl IdentityMatch {
    /// True when the WordPress account uses an alias rather than the primary
    /// address of the reference file.
    pub fn is_alias_match(&self, normalizer: &Normalizer) -> bool {
        self.users
            .iter()
            .any(|user| normalizer.normalize(&user.email) != self.identity.primary().key())
    }
}

/// Outcome of a reconciliation, everything the commands need to report or act.
#[derive(Debug, Clone, Default)]
pub struct Plan {
    /// Identities matched by exactly one WordPress user.
    pub matched: Vec<IdentityMatch>,
    /// Identities matched by several WordPress users: needs a human decision.
    pub ambiguous: Vec<IdentityMatch>,
    /// Identities with no WordPress user: to create.
    pub missing: Vec<Identity>,
    /// WordPress users absent from the reference file: to delete.
    pub extra: Vec<WpUser>,
    /// WordPress users absent from the reference file but shielded from
    /// deletion, with the reason why.
    pub protected: Vec<(WpUser, String)>,
}

/// Rules deciding which surplus users must never be deleted.
#[derive(Debug, Clone, Default)]
pub struct Protection {
    /// Normalized e-mail keys that must be kept.
    pub emails: Vec<String>,
    /// Keep administrators even when they are not in the reference file.
    pub keep_administrators: bool,
}

impl Protection {
    fn reason(&self, user: &WpUser, normalizer: &Normalizer) -> Option<String> {
        let key = normalizer.normalize(&user.email);
        if self.emails.contains(&key) {
            return Some("protected e-mail".to_string());
        }
        if self.keep_administrators && user.is_administrator() {
            return Some("administrator".to_string());
        }
        None
    }
}

pub fn reconcile(
    reference: &Reference,
    users: &[WpUser],
    normalizer: &Normalizer,
    protection: &Protection,
) -> Plan {
    // Normalized WordPress e-mail -> identity index it belongs to.
    let mut owner: HashMap<usize, Vec<WpUser>> = HashMap::new();
    let mut plan = Plan::default();

    for user in users {
        let key = normalizer.normalize(&user.email);
        match reference
            .identities
            .iter()
            .position(|identity| identity.matches_key(&key))
        {
            Some(index) => owner.entry(index).or_default().push(user.clone()),
            None => match protection.reason(user, normalizer) {
                Some(reason) => plan.protected.push((user.clone(), reason)),
                None => plan.extra.push(user.clone()),
            },
        }
    }

    for (index, identity) in reference.identities.iter().enumerate() {
        match owner.remove(&index) {
            None => plan.missing.push(identity.clone()),
            Some(users) if users.len() == 1 => plan.matched.push(IdentityMatch {
                identity: identity.clone(),
                users,
            }),
            Some(users) => plan.ambiguous.push(IdentityMatch {
                identity: identity.clone(),
                users,
            }),
        }
    }

    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reference::parse;
    use std::path::PathBuf;

    fn reference_from(content: &str, normalizer: &Normalizer) -> Reference {
        Reference {
            path: PathBuf::from("<test>"),
            identities: parse(content, normalizer).unwrap(),
        }
    }

    fn user(id: u64, email: &str, roles: &[&str]) -> WpUser {
        WpUser {
            id,
            username: email.split('@').next().unwrap().to_string(),
            name: email.to_string(),
            email: email.to_string(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        }
    }

    #[test]
    fn splits_missing_matched_and_extra() {
        let normalizer = Normalizer::default();
        let reference = reference_from("a@example.com\nb@example.com\n", &normalizer);
        let users = vec![
            user(1, "a@example.com", &["subscriber"]),
            user(2, "c@example.com", &["subscriber"]),
        ];

        let plan = reconcile(&reference, &users, &normalizer, &Protection::default());

        assert_eq!(plan.matched.len(), 1);
        assert_eq!(plan.matched[0].users[0].id, 1);
        assert_eq!(plan.missing.len(), 1);
        assert_eq!(plan.missing[0].primary().raw(), "b@example.com");
        assert_eq!(plan.extra.len(), 1);
        assert_eq!(plan.extra[0].id, 2);
    }

    #[test]
    fn matches_an_alias_from_the_same_line() {
        let normalizer = Normalizer::default();
        let reference = reference_from("new@example.com old@legacy.com\n", &normalizer);
        let users = vec![user(7, "old@legacy.com", &["author"])];

        let plan = reconcile(&reference, &users, &normalizer, &Protection::default());

        assert!(plan.missing.is_empty());
        assert!(plan.extra.is_empty());
        assert_eq!(plan.matched.len(), 1);
        assert!(plan.matched[0].is_alias_match(&normalizer));
    }

    #[test]
    fn reports_several_accounts_for_one_identity_as_ambiguous() {
        let normalizer = Normalizer::default();
        let reference = reference_from("new@example.com old@legacy.com\n", &normalizer);
        let users = vec![
            user(1, "new@example.com", &["subscriber"]),
            user(2, "old@legacy.com", &["subscriber"]),
        ];

        let plan = reconcile(&reference, &users, &normalizer, &Protection::default());

        assert!(plan.matched.is_empty());
        assert_eq!(plan.ambiguous.len(), 1);
        assert_eq!(plan.ambiguous[0].users.len(), 2);
        assert!(plan.extra.is_empty());
    }

    #[test]
    fn shields_protected_emails_and_administrators() {
        let normalizer = Normalizer::default();
        let reference = reference_from("a@example.com\n", &normalizer);
        let users = vec![
            user(1, "a@example.com", &["subscriber"]),
            user(2, "admin@example.com", &["administrator"]),
            user(3, "keep@example.com", &["editor"]),
            user(4, "gone@example.com", &["subscriber"]),
        ];
        let protection = Protection {
            emails: vec!["keep@example.com".to_string()],
            keep_administrators: true,
        };

        let plan = reconcile(&reference, &users, &normalizer, &protection);

        assert_eq!(plan.extra.len(), 1);
        assert_eq!(plan.extra[0].id, 4);
        let protected: Vec<u64> = plan.protected.iter().map(|(u, _)| u.id).collect();
        assert_eq!(protected, vec![2, 3]);
    }
}
