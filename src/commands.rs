//! Implementation of the subcommands.

use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow};

use crate::aliases::AliasMap;
use crate::cli::{CreateArgs, DeleteArgs, GlobalArgs};
use crate::prompt::{confirm, confirm_phrase};
use crate::provision::new_user_for;
use crate::reconcile::{Plan, Protection, reconcile, users_for};
use crate::reference::{Email, Identity, Normalizer, Reference};
use crate::report;
use crate::wp::{NewUser, WpClient, WpUser};

/// Everything the reconciling commands need: the reference file and a client.
struct Session {
    reference: Reference,
    aliases: AliasMap,
    client: WpClient,
    normalizer: Normalizer,
    protection: Protection,
    users: Vec<WpUser>,
    plan: Plan,
}

pub fn normalizer(global: &GlobalArgs) -> Normalizer {
    Normalizer {
        strip_plus_tags: global.strip_plus_tags,
    }
}

/// The alias file is optional: without one, every address stands for itself.
pub fn load_aliases(global: &GlobalArgs) -> Result<AliasMap> {
    match global.aliases.as_ref() {
        None => Ok(AliasMap::empty()),
        Some(path) => Ok(AliasMap::load(path, &normalizer(global))?),
    }
}

pub fn load_reference(global: &GlobalArgs, aliases: &AliasMap) -> Result<Reference> {
    let path = global.reference.as_ref().ok_or_else(|| {
        anyhow!("no reference file: pass --reference <FILE> or set WP_REFERENCE_FILE")
    })?;
    Ok(Reference::load(path, &normalizer(global), aliases)?)
}

fn client(global: &GlobalArgs) -> Result<WpClient> {
    let url = required(&global.wp_url, "--wp-url", "WP_URL")?;
    let user = required(&global.wp_user, "--wp-user", "WP_USER")?;
    let password = required(
        &global.wp_app_password,
        "--wp-app-password",
        "WP_APP_PASSWORD",
    )?;
    Ok(WpClient::new(url, user, password, global.timeout())?)
}

fn required<'a>(value: &'a Option<String>, flag: &str, env: &str) -> Result<&'a str> {
    match value {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(anyhow!(
            "missing WordPress credentials: pass {flag} or set {env}"
        )),
    }
}

async fn open_session(global: &GlobalArgs) -> Result<Session> {
    let aliases = load_aliases(global)?;
    let reference = load_reference(global, &aliases)?;
    let client = client(global)?;
    let normalizer = normalizer(global);

    let me = client
        .current_user()
        .await
        .context("could not identify the authenticated WordPress user")?;
    let users = client
        .list_users()
        .await
        .context("could not list the WordPress users")?;

    // The account we authenticate with is always kept, on top of the
    // explicitly protected ones. Protection is expressed in classes, so an
    // alias of a protected address is protected too.
    let mut protected: Vec<String> = global
        .protect
        .iter()
        .filter(|email| !email.trim().is_empty())
        .map(|email| class_of(&aliases, &normalizer, email))
        .collect();
    protected.push(class_of(&aliases, &normalizer, &me.email));

    let protection = Protection {
        classes: protected,
        keep_administrators: !global.delete_administrators,
    };
    let plan = reconcile(&reference, &users, &normalizer, &aliases, &protection);

    println!(
        "Site      : {}\nAuthenticated as: {} <{}>\nReference : {} ({} identities)\nAliases   : {}",
        global.wp_url.as_deref().unwrap_or("-"),
        me.username,
        me.email,
        reference.path.display(),
        reference.identities.len(),
        match aliases.path() {
            Some(path) => format!("{} ({} groups)", path.display(), aliases.groups().len()),
            None => "none (pass --aliases <FILE> to map alternative addresses)".to_string(),
        },
    );

    Ok(Session {
        reference,
        aliases,
        client,
        normalizer,
        protection,
        users,
        plan,
    })
}

pub fn list_reference(global: &GlobalArgs) -> Result<()> {
    let aliases = load_aliases(global)?;
    let reference = load_reference(global, &aliases)?;
    report::heading(&format!("Reference file {}", reference.path.display()));
    report::identities_list(&reference.identities, &aliases);
    Ok(())
}

pub fn list_aliases(global: &GlobalArgs) -> Result<()> {
    let aliases = load_aliases(global)?;
    let title = match aliases.path() {
        Some(path) => format!("Alias file {}", path.display()),
        None => "No alias file (pass --aliases <FILE> or set WP_ALIAS_FILE)".to_string(),
    };
    report::heading(&title);
    report::alias_groups(aliases.groups());
    Ok(())
}

/// Resolves a raw address to the key identifying its owner.
fn class_of(aliases: &AliasMap, normalizer: &Normalizer, email: &str) -> String {
    let key = normalizer.normalize(email);
    aliases.class_of(&key).to_string()
}

pub async fn list_users(global: &GlobalArgs) -> Result<()> {
    let client = client(global)?;
    let users = client
        .list_users()
        .await
        .context("could not list the WordPress users")?;
    report::heading("WordPress users");
    report::users_table(&users);
    Ok(())
}

pub async fn status(global: &GlobalArgs) -> Result<()> {
    let session = open_session(global).await?;
    let plan = &session.plan;

    report::heading("Missing users (in the reference file, not in WordPress)");
    report::identities_list(&plan.missing, &session.aliases);

    report::heading("Surplus users (in WordPress, not in the reference file)");
    report::users_table(&plan.extra);

    report::protected_details(&plan.protected);
    report::ambiguous_details(&plan.ambiguous, &session.aliases);
    report::matched_details(plan, &session.normalizer);
    report::plan_summary(plan, &session.normalizer);

    println!(
        "\n{} WordPress user(s) in total, {} reference identity/identities.",
        session.users.len(),
        session.reference.identities.len()
    );
    Ok(())
}

pub async fn list_missing(global: &GlobalArgs) -> Result<()> {
    let session = open_session(global).await?;
    report::heading("Missing users (to create)");
    report::identities_list(&session.plan.missing, &session.aliases);
    report::ambiguous_details(&session.plan.ambiguous, &session.aliases);
    Ok(())
}

pub async fn list_extra(global: &GlobalArgs) -> Result<()> {
    let session = open_session(global).await?;
    report::heading("Surplus users (to delete)");
    report::users_table(&session.plan.extra);
    report::protected_details(&session.plan.protected);
    Ok(())
}

pub async fn create_missing(global: &GlobalArgs, args: &CreateArgs) -> Result<()> {
    let session = open_session(global).await?;
    create_from_session(&session, global, args).await
}

/// Creates a single user from an address given on the command line. The
/// reference file is not required here: the address itself says who to create.
pub async fn create_user(global: &GlobalArgs, email: &str, args: &CreateArgs) -> Result<()> {
    let normalizer = normalizer(global);
    let aliases = load_aliases(global)?;
    let email = Email::new(email, &normalizer)
        .ok_or_else(|| anyhow!("`{email}` is not a valid e-mail address"))?;
    let identity = Identity::ad_hoc(email, &aliases);

    let client = client(global)?;
    let users = client
        .list_users()
        .await
        .context("could not list the WordPress users")?;

    report::heading("User to create");
    println!("{}", identity.display(&aliases));

    // An account may already exist under any address of the alias group.
    let existing = users_for(&identity, &users, &normalizer, &aliases);
    if !existing.is_empty() {
        println!("\nThis person already has a WordPress account, nothing to create:");
        report::users_table(&existing);
        return Ok(());
    }

    // The reference file stays the source of truth for who must exist: an
    // address missing from it would be listed as surplus by the next `sync`.
    if let Some(path) = global.reference.as_ref() {
        let reference = load_reference(global, &aliases)?;
        if !reference
            .identities
            .iter()
            .any(|other| other.class == identity.class)
        {
            println!(
                "\nWarning: this address is not listed in the reference file {}.\n\
                 Add it there, or `delete-extra` will propose the new account for deletion.",
                path.display()
            );
        }
    }

    let question = format!(
        "Create 1 user with role `{}` on {}?",
        args.role,
        global.wp_url.as_deref().unwrap_or("this site")
    );
    if !confirm(&question, global.yes)? {
        println!("Aborted, nothing was created.");
        return Ok(());
    }

    let mut taken: HashSet<String> = users
        .iter()
        .map(|user| user.username.to_lowercase())
        .collect();
    let (new_user, password) = new_user_for(&identity, &args.role, &mut taken);
    let created = create_one(&client, &new_user, &password, args.show_passwords).await?;
    if created && !args.show_passwords {
        println!(
            "The password was generated randomly and not displayed: ask the new user to \
             go through the WordPress \"lost password\" flow (re-run with --show-passwords \
             to print it)."
        );
    }
    Ok(())
}

/// Creates one user and reports the outcome. Returns whether it was created.
async fn create_one(
    client: &WpClient,
    new_user: &NewUser,
    password: &str,
    show_passwords: bool,
) -> Result<bool> {
    match client.create_user(new_user).await {
        Ok(user) => {
            let secret = if show_passwords {
                format!("  password: {password}")
            } else {
                String::new()
            };
            println!(
                "created id {:<6} {:<24} {}{}",
                user.id, new_user.username, new_user.email, secret
            );
            Ok(true)
        }
        Err(error) => Err(anyhow!("could not create {} : {error}", new_user.email)),
    }
}

pub async fn delete_extra(global: &GlobalArgs, args: &DeleteArgs) -> Result<()> {
    let session = open_session(global).await?;
    delete_from_session(&session, global, args).await
}

pub async fn sync(global: &GlobalArgs, create: &CreateArgs, delete: &DeleteArgs) -> Result<()> {
    let session = open_session(global).await?;
    report::plan_summary(&session.plan, &session.normalizer);
    report::ambiguous_details(&session.plan.ambiguous, &session.aliases);

    create_from_session(&session, global, create).await?;
    delete_from_session(&session, global, delete).await
}

async fn create_from_session(
    session: &Session,
    global: &GlobalArgs,
    args: &CreateArgs,
) -> Result<()> {
    report::heading("Users to create");
    report::identities_list(&session.plan.missing, &session.aliases);
    if session.plan.missing.is_empty() {
        return Ok(());
    }

    let question = format!(
        "Create {} user(s) with role `{}` on {}?",
        session.plan.missing.len(),
        args.role,
        global.wp_url.as_deref().unwrap_or("this site")
    );
    if !confirm(&question, global.yes)? {
        println!("Aborted, nothing was created.");
        return Ok(());
    }

    let mut taken: HashSet<String> = session
        .users
        .iter()
        .map(|user| user.username.to_lowercase())
        .collect();

    let mut created = 0usize;
    let mut failed = 0usize;
    for identity in &session.plan.missing {
        let (new_user, password) = new_user_for(identity, &args.role, &mut taken);
        match session.client.create_user(&new_user).await {
            Ok(user) => {
                created += 1;
                let secret = if args.show_passwords {
                    format!("  password: {password}")
                } else {
                    String::new()
                };
                println!(
                    "created id {:<6} {:<24} {}{}",
                    user.id, new_user.username, new_user.email, secret
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!("FAILED  {} : {error}", new_user.email);
            }
        }
    }

    println!("\n{created} user(s) created, {failed} failure(s).");
    if created > 0 && !args.show_passwords {
        println!(
            "Passwords were generated randomly and not displayed: ask the new users to \
             go through the WordPress \"lost password\" flow (re-run with --show-passwords \
             to print them)."
        );
    }
    if failed > 0 {
        return Err(anyhow!("{failed} user(s) could not be created"));
    }
    Ok(())
}

async fn delete_from_session(
    session: &Session,
    global: &GlobalArgs,
    args: &DeleteArgs,
) -> Result<()> {
    report::heading("Users to delete");
    report::users_table(&session.plan.extra);
    report::protected_details(&session.plan.protected);
    if session.plan.extra.is_empty() {
        return Ok(());
    }

    let fate = match args.reassign {
        Some(id) => format!("their content will be re-assigned to user {id}"),
        None => "their content will be deleted as well".to_string(),
    };
    println!("\nThis deletion is permanent and {fate}.");
    if session.protection.keep_administrators {
        println!("Administrators are kept (pass --delete-administrators to include them).");
    } else {
        println!("WARNING: --delete-administrators is set, administrators may be deleted.");
    }

    let question = format!(
        "Permanently delete {} user(s) from {}?",
        session.plan.extra.len(),
        global.wp_url.as_deref().unwrap_or("this site")
    );
    if !confirm_phrase(&question, "delete", global.yes)? {
        println!("Aborted, nothing was deleted.");
        return Ok(());
    }

    let mut deleted = 0usize;
    let mut failed = 0usize;
    for user in &session.plan.extra {
        match session.client.delete_user(user.id, args.reassign).await {
            Ok(()) => {
                deleted += 1;
                println!(
                    "deleted id {:<6} {:<24} {}",
                    user.id, user.username, user.email
                );
            }
            Err(error) => {
                failed += 1;
                eprintln!("FAILED  id {} {} : {error}", user.id, user.email);
            }
        }
    }

    println!("\n{deleted} user(s) deleted, {failed} failure(s).");
    if failed > 0 {
        return Err(anyhow!("{failed} user(s) could not be deleted"));
    }
    Ok(())
}
