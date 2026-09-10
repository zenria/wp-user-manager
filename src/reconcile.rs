//! Matching of the reference file against the WordPress user base.
//!
//! Both sides are compared on their *class*: the normalized address, resolved
//! through the alias map. Two addresses of the same alias group therefore
//! designate the same person, whichever one WordPress happens to store.

use std::collections::HashMap;

use crate::aliases::AliasMap;
use crate::reference::{Identity, Normalizer, Reference};
use crate::wp::WpUser;

/// A reference identity and the WordPress user(s) whose e-mail resolves to the
/// same person.
#[derive(Debug, Clone)]
pub struct IdentityMatch {
    pub identity: Identity,
    pub users: Vec<WpUser>,
}

impl IdentityMatch {
    /// True when the WordPress account uses an alias rather than the address
    /// listed in the reference file.
    pub fn is_alias_match(&self, normalizer: &Normalizer) -> bool {
        self.users
            .iter()
            .any(|user| normalizer.normalize(&user.email) != self.identity.email().key())
    }
}

/// Outcome of a reconciliation: everything the commands need to report or act.
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
    /// Classes (normalized, alias-resolved keys) that must be kept.
    pub classes: Vec<String>,
    /// Keep administrators even when they are not in the reference file.
    pub keep_administrators: bool,
}

impl Protection {
    fn reason(&self, user: &WpUser, class: &str) -> Option<String> {
        if self.classes.iter().any(|protected| protected == class) {
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
    aliases: &AliasMap,
    protection: &Protection,
) -> Plan {
    // class key -> index of the reference identity holding it
    let index_of: HashMap<&str, usize> = reference
        .identities
        .iter()
        .enumerate()
        .map(|(index, identity)| (identity.class.as_str(), index))
        .collect();

    let mut owned: HashMap<usize, Vec<WpUser>> = HashMap::new();
    let mut plan = Plan::default();

    for user in users {
        let key = normalizer.normalize(&user.email);
        let class = aliases.class_of(&key);
        match index_of.get(class) {
            Some(&index) => owned.entry(index).or_default().push(user.clone()),
            None => match protection.reason(user, class) {
                Some(reason) => plan.protected.push((user.clone(), reason)),
                None => plan.extra.push(user.clone()),
            },
        }
    }

    for (index, identity) in reference.identities.iter().enumerate() {
        match owned.remove(&index) {
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

    fn aliases(content: &str) -> AliasMap {
        AliasMap::parse(content, &Normalizer::default()).unwrap()
    }

    fn reference_from(content: &str, aliases: &AliasMap) -> Reference {
        Reference {
            path: PathBuf::from("<test>"),
            identities: parse(content, &Normalizer::default(), aliases).unwrap(),
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
        let aliases = AliasMap::empty();
        let reference = reference_from("a@example.com\nb@example.com\n", &aliases);
        let users = vec![
            user(1, "a@example.com", &["subscriber"]),
            user(2, "c@example.com", &["subscriber"]),
        ];

        let plan = reconcile(
            &reference,
            &users,
            &normalizer,
            &aliases,
            &Protection::default(),
        );

        assert_eq!(plan.matched.len(), 1);
        assert_eq!(plan.matched[0].users[0].id, 1);
        assert_eq!(plan.missing.len(), 1);
        assert_eq!(plan.missing[0].email().raw(), "b@example.com");
        assert_eq!(plan.extra.len(), 1);
        assert_eq!(plan.extra[0].id, 2);
    }

    #[test]
    fn matches_a_wordpress_account_registered_under_an_alias() {
        let normalizer = Normalizer::default();
        let aliases = aliases("new@example.com old@legacy.com\n");
        let reference = reference_from("new@example.com\n", &aliases);
        let users = vec![user(7, "old@legacy.com", &["author"])];

        let plan = reconcile(
            &reference,
            &users,
            &normalizer,
            &aliases,
            &Protection::default(),
        );

        assert!(
            plan.missing.is_empty(),
            "the account exists under its alias"
        );
        assert!(plan.extra.is_empty(), "an alias is not a surplus user");
        assert_eq!(plan.matched.len(), 1);
        assert!(plan.matched[0].is_alias_match(&normalizer));
    }

    #[test]
    fn matches_through_an_alias_even_when_the_reference_lists_the_old_address() {
        let normalizer = Normalizer::default();
        let aliases = aliases("old@legacy.com new@example.com\n");
        let reference = reference_from("old@legacy.com\n", &aliases);
        let users = vec![user(7, "new@example.com", &["author"])];

        let plan = reconcile(
            &reference,
            &users,
            &normalizer,
            &aliases,
            &Protection::default(),
        );

        assert_eq!(plan.matched.len(), 1, "the mapping works both ways");
        assert!(plan.missing.is_empty());
        assert!(plan.extra.is_empty());
    }

    #[test]
    fn an_alias_never_becomes_a_user_to_create() {
        let normalizer = Normalizer::default();
        let aliases = aliases("a@example.com a@old.com\nunused@example.com unused@old.com\n");
        let reference = reference_from("a@example.com\n", &aliases);
        let users = vec![user(1, "a@old.com", &["subscriber"])];

        let plan = reconcile(
            &reference,
            &users,
            &normalizer,
            &aliases,
            &Protection::default(),
        );

        assert!(
            plan.missing.is_empty(),
            "aliases are a mapping, not reference entries"
        );
        assert_eq!(plan.matched.len(), 1);
    }

    #[test]
    fn reports_several_accounts_for_one_identity_as_ambiguous() {
        let normalizer = Normalizer::default();
        let aliases = aliases("new@example.com old@legacy.com\n");
        let reference = reference_from("new@example.com\n", &aliases);
        let users = vec![
            user(1, "new@example.com", &["subscriber"]),
            user(2, "old@legacy.com", &["subscriber"]),
        ];

        let plan = reconcile(
            &reference,
            &users,
            &normalizer,
            &aliases,
            &Protection::default(),
        );

        assert!(plan.matched.is_empty());
        assert_eq!(plan.ambiguous.len(), 1);
        assert_eq!(plan.ambiguous[0].users.len(), 2);
        assert!(plan.extra.is_empty());
    }

    #[test]
    fn shields_protected_emails_and_administrators() {
        let normalizer = Normalizer::default();
        let aliases = AliasMap::empty();
        let reference = reference_from("a@example.com\n", &aliases);
        let users = vec![
            user(1, "a@example.com", &["subscriber"]),
            user(2, "admin@example.com", &["administrator"]),
            user(3, "keep@example.com", &["editor"]),
            user(4, "gone@example.com", &["subscriber"]),
        ];
        let protection = Protection {
            classes: vec!["keep@example.com".to_string()],
            keep_administrators: true,
        };

        let plan = reconcile(&reference, &users, &normalizer, &aliases, &protection);

        assert_eq!(plan.extra.len(), 1);
        assert_eq!(plan.extra[0].id, 4);
        let protected: Vec<u64> = plan.protected.iter().map(|(u, _)| u.id).collect();
        assert_eq!(protected, vec![2, 3]);
    }

    #[test]
    fn protects_a_user_through_an_alias_of_the_protected_address() {
        let normalizer = Normalizer::default();
        let aliases = aliases("owner@example.com owner@old.com\n");
        let reference = reference_from("a@example.com\n", &aliases);
        let users = vec![user(9, "owner@old.com", &["editor"])];
        let protection = Protection {
            classes: vec![aliases.class_of("owner@example.com").to_string()],
            keep_administrators: false,
        };

        let plan = reconcile(&reference, &users, &normalizer, &aliases, &protection);

        assert!(plan.extra.is_empty());
        assert_eq!(plan.protected.len(), 1);
    }
}
