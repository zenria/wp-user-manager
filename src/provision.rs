//! Derivation of the WordPress account details for a user to be created.

use std::collections::HashSet;

use rand::distributions::{Alphanumeric, Distribution, Uniform};

use crate::reference::Identity;
use crate::wp::NewUser;

const USERNAME_MAX_LEN: usize = 60;
const PASSWORD_LEN: usize = 24;
const PASSWORD_SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{}";

/// Builds the payload for an identity, keeping usernames unique against the
/// ones already taken (`taken` is updated with the chosen username).
pub fn new_user_for(
    identity: &Identity,
    role: &str,
    taken: &mut HashSet<String>,
) -> (NewUser, String) {
    let email = identity.email();
    let base = sanitize_username(email.local_part());
    let username = unique_username(&base, taken);
    taken.insert(username.clone());
    let password = generate_password();

    let new_user = NewUser {
        username,
        email: email.raw().to_string(),
        name: display_name(email.local_part()),
        password: password.clone(),
        roles: vec![role.to_string()],
    };
    (new_user, password)
}

fn sanitize_username(local_part: &str) -> String {
    let cleaned: String = local_part
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '.'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    let cleaned: String = cleaned.chars().take(USERNAME_MAX_LEN).collect();
    if cleaned.is_empty() {
        "user".to_string()
    } else {
        cleaned
    }
}

fn unique_username(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    for suffix in 2..1000 {
        let candidate = format!("{base}{suffix}");
        if !taken.contains(&candidate) {
            return candidate;
        }
    }
    let suffix = Uniform::from(1000..u32::MAX).sample(&mut rand::thread_rng());
    format!("{base}{suffix}")
}

fn display_name(local_part: &str) -> String {
    local_part
        .split(['.', '_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A random password, strong enough that nobody is expected to reuse it: new
/// users are meant to go through the "lost password" flow.
fn generate_password() -> String {
    let mut rng = rand::thread_rng();
    let mut password: String = Alphanumeric
        .sample_iter(&mut rng)
        .take(PASSWORD_LEN - 4)
        .map(char::from)
        .collect();
    let symbol = Uniform::from(0..PASSWORD_SYMBOLS.len());
    for _ in 0..4 {
        password.push(PASSWORD_SYMBOLS[symbol.sample(&mut rng)] as char);
    }
    password
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aliases::AliasMap;
    use crate::reference::{Normalizer, parse};

    fn identity(line: &str) -> Identity {
        parse(line, &Normalizer::default(), &AliasMap::empty())
            .unwrap()
            .remove(0)
    }

    #[test]
    fn derives_username_and_display_name_from_the_reference_email() {
        let mut taken = HashSet::new();
        let (user, _) = new_user_for(
            &identity("Jane.Doe@example.com\n"),
            "subscriber",
            &mut taken,
        );
        assert_eq!(user.username, "jane.doe");
        assert_eq!(user.name, "Jane Doe");
        assert_eq!(user.email, "Jane.Doe@example.com");
        assert_eq!(user.roles, vec!["subscriber".to_string()]);
    }

    #[test]
    fn suffixes_usernames_already_taken() {
        let mut taken = HashSet::from(["jane".to_string()]);
        let (user, _) = new_user_for(&identity("jane@example.com\n"), "subscriber", &mut taken);
        assert_eq!(user.username, "jane2");
        assert!(taken.contains("jane2"));
    }

    #[test]
    fn replaces_characters_wordpress_would_reject() {
        let mut taken = HashSet::new();
        let (user, _) = new_user_for(&identity("a+b/c@example.com\n"), "subscriber", &mut taken);
        assert_eq!(user.username, "a.b.c");
    }

    #[test]
    fn generates_a_long_random_password() {
        let first = generate_password();
        assert_eq!(first.chars().count(), PASSWORD_LEN);
        assert_ne!(first, generate_password());
    }
}
