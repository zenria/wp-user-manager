//! Implementation of the subcommands.

use std::collections::HashSet;

use anyhow::{Context as _, Result, anyhow};

use crate::cli::{CreateArgs, DeleteArgs, GlobalArgs};
use crate::prompt::{confirm, confirm_phrase};
use crate::provision::new_user_for;
use crate::reconcile::{Plan, Protection, reconcile};
use crate::reference::{Normalizer, Reference};
use crate::report;
use crate::wp::{WpClient, WpUser};

/// Everything the reconciling commands need: the reference file and a client.
struct Session {
    reference: Reference,
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

pub fn load_reference(global: &GlobalArgs) -> Result<Reference> {
    let path = global.reference.as_ref().ok_or_else(|| {
        anyhow!("no reference file: pass --reference <FILE> or set WP_REFERENCE_FILE")
    })?;
    Ok(Reference::load(path, &normalizer(global))?)
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
    let reference = load_reference(global)?;
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
    // explicitly protected ones.
    let mut protected: Vec<String> = global
        .protect
        .iter()
        .filter(|email| !email.trim().is_empty())
        .map(|email| normalizer.normalize(email))
        .collect();
    protected.push(normalizer.normalize(&me.email));

    let protection = Protection {
        emails: protected,
        keep_administrators: !global.delete_administrators,
    };
    let plan = reconcile(&reference, &users, &normalizer, &protection);

    println!(
        "Site      : {}\nAuthenticated as: {} <{}>\nReference : {} ({} identities, {} e-mails)",
        global.wp_url.as_deref().unwrap_or("-"),
        me.username,
        me.email,
        reference.path.display(),
        reference.identities.len(),
        reference.email_count(),
    );

    Ok(Session {
        reference,
        client,
        normalizer,
        protection,
        users,
        plan,
    })
}

pub fn list_reference(global: &GlobalArgs) -> Result<()> {
    let reference = load_reference(global)?;
    report::heading(&format!("Reference file {}", reference.path.display()));
    report::identities_list(&reference.identities);
    Ok(())
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
    report::identities_list(&plan.missing);

    report::heading("Surplus users (in WordPress, not in the reference file)");
    report::users_table(&plan.extra);

    report::protected_details(&plan.protected);
    report::ambiguous_details(&plan.ambiguous);
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
    report::identities_list(&session.plan.missing);
    report::ambiguous_details(&session.plan.ambiguous);
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

pub async fn delete_extra(global: &GlobalArgs, args: &DeleteArgs) -> Result<()> {
    let session = open_session(global).await?;
    delete_from_session(&session, global, args).await
}

pub async fn sync(global: &GlobalArgs, create: &CreateArgs, delete: &DeleteArgs) -> Result<()> {
    let session = open_session(global).await?;
    report::plan_summary(&session.plan, &session.normalizer);
    report::ambiguous_details(&session.plan.ambiguous);

    create_from_session(&session, global, create).await?;
    delete_from_session(&session, global, delete).await
}

async fn create_from_session(
    session: &Session,
    global: &GlobalArgs,
    args: &CreateArgs,
) -> Result<()> {
    report::heading("Users to create");
    report::identities_list(&session.plan.missing);
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
                    "deleted id {:<6} {:<28} {}",
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
