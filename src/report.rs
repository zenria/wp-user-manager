//! Plain text rendering of users, identities and reconciliation plans.

use crate::reconcile::{IdentityMatch, Plan};
use crate::reference::{Identity, Normalizer};
use crate::wp::WpUser;

pub fn heading(title: &str) {
    println!("\n{title}");
    println!("{}", "-".repeat(title.chars().count()));
}

pub fn users_table(users: &[WpUser]) {
    if users.is_empty() {
        println!("(none)");
        return;
    }
    println!(
        "{:>6}  {:<24}  {:<34}  {:<24}  ROLES",
        "ID", "USERNAME", "E-MAIL", "NAME"
    );
    for user in users {
        println!(
            "{:>6}  {:<24}  {:<34}  {:<24}  {}",
            user.id,
            user.username,
            user.email,
            user.name,
            user.roles_display()
        );
    }
    println!("\n{} user(s)", users.len());
}

pub fn identities_list(identities: &[Identity]) {
    if identities.is_empty() {
        println!("(none)");
        return;
    }
    for identity in identities {
        let aliases = identity.aliases();
        if aliases.is_empty() {
            println!("line {:>4}  {}", identity.line, identity.primary().raw());
        } else {
            let aliases: Vec<&str> = aliases.iter().map(|email| email.raw()).collect();
            println!(
                "line {:>4}  {}  (also: {})",
                identity.line,
                identity.primary().raw(),
                aliases.join(", ")
            );
        }
    }
    println!("\n{} identity/identities", identities.len());
}

pub fn plan_summary(plan: &Plan, normalizer: &Normalizer) {
    heading("Summary");
    let aliased = plan
        .matched
        .iter()
        .filter(|m| m.is_alias_match(normalizer))
        .count();
    println!("  matched            : {}", plan.matched.len());
    println!("    of which aliases : {aliased}");
    println!("  to create (missing): {}", plan.missing.len());
    println!("  to delete (surplus): {}", plan.extra.len());
    println!("  protected          : {}", plan.protected.len());
    println!("  ambiguous          : {}", plan.ambiguous.len());
}

pub fn matched_details(plan: &Plan, normalizer: &Normalizer) {
    heading("Matched users");
    if plan.matched.is_empty() {
        println!("(none)");
        return;
    }
    for entry in &plan.matched {
        let user = &entry.users[0];
        let marker = if entry.is_alias_match(normalizer) {
            "  [alias]"
        } else {
            ""
        };
        println!(
            "{:>6}  {:<28}  {:<36}  <- line {}{}",
            user.id, user.username, user.email, entry.identity.line, marker
        );
    }
}

pub fn ambiguous_details(entries: &[IdentityMatch]) {
    heading("Ambiguous identities (several WordPress accounts on one line)");
    if entries.is_empty() {
        println!("(none)");
        return;
    }
    for entry in entries {
        println!("line {}: {}", entry.identity.line, entry.identity.display());
        for user in &entry.users {
            println!(
                "    -> id {:<6} {:<24} {}",
                user.id, user.username, user.email
            );
        }
    }
    println!(
        "\nNothing is created or deleted for these lines: merge the accounts in \
         WordPress, or split the line."
    );
}

pub fn protected_details(protected: &[(WpUser, String)]) {
    heading("Surplus users kept (protected)");
    if protected.is_empty() {
        println!("(none)");
        return;
    }
    for (user, reason) in protected {
        println!(
            "{:>6}  {:<24}  {:<34}  {reason}",
            user.id, user.username, user.email
        );
    }
}
