//! Command line surface. Every option can also be provided through an
//! environment variable (and therefore through a `.env` file).

use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "wp-user-manager",
    version,
    about = "Reconcile the users of a WordPress site with a reference list of e-mail addresses",
    long_about = None,
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Args)]
pub struct GlobalArgs {
    /// Base URL of the WordPress site, e.g. https://example.com
    #[arg(long, env = "WP_URL", global = true)]
    pub wp_url: Option<String>,

    /// WordPress login owning the application password
    #[arg(long, env = "WP_USER", global = true)]
    pub wp_user: Option<String>,

    /// Application password (Users > Profile > Application Passwords)
    #[arg(long, env = "WP_APP_PASSWORD", global = true, hide_env_values = true)]
    pub wp_app_password: Option<String>,

    /// Reference file: one identity per line, several e-mails per line allowed
    #[arg(
        short = 'f',
        long = "reference",
        env = "WP_REFERENCE_FILE",
        global = true
    )]
    pub reference: Option<PathBuf>,

    /// Alias file: one group of equivalent addresses per line (order-agnostic)
    #[arg(short = 'a', long = "aliases", env = "WP_ALIAS_FILE", global = true)]
    pub aliases: Option<PathBuf>,

    /// Treat `user+tag@host` as `user@host` when comparing addresses
    #[arg(long, env = "WP_STRIP_PLUS_TAGS", global = true)]
    pub strip_plus_tags: bool,

    /// E-mail addresses that must never be deleted (repeatable)
    #[arg(
        long = "protect",
        env = "WP_PROTECTED_EMAILS",
        value_delimiter = ',',
        global = true
    )]
    pub protect: Vec<String>,

    /// Allow deleting WordPress administrators missing from the reference file
    #[arg(long, env = "WP_DELETE_ADMINISTRATORS", global = true)]
    pub delete_administrators: bool,

    /// Answer yes to every confirmation prompt (for non-interactive runs)
    #[arg(short = 'y', long, env = "WP_ASSUME_YES", global = true)]
    pub yes: bool,

    /// HTTP timeout in seconds
    #[arg(long, env = "WP_TIMEOUT_SECONDS", default_value_t = 30, global = true)]
    pub timeout: u64,
}

impl GlobalArgs {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout.max(1))
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the identities read from the reference file (no WordPress call)
    ListReference,

    /// Show the alias groups read from the alias file (no WordPress call)
    ListAliases,

    /// List the users of the WordPress site
    ListUsers,

    /// Full reconciliation report: matched, missing, surplus, ambiguous
    Status,

    /// List the reference identities that have no WordPress user
    ListMissing,

    /// List the WordPress users absent from the reference file
    ListExtra,

    /// Create the missing users, after confirmation
    CreateMissing(CreateArgs),

    /// Create a single user from an e-mail address, after confirmation
    CreateUser(CreateUserArgs),

    /// Delete the surplus users, after confirmation
    DeleteExtra(DeleteArgs),

    /// Create the missing users then delete the surplus ones, each confirmed
    Sync(SyncArgs),
}

#[derive(Debug, Args)]
pub struct CreateArgs {
    /// Role given to created users
    #[arg(long, env = "WP_DEFAULT_ROLE", default_value = "subscriber")]
    pub role: String,

    /// Print the generated passwords instead of hiding them
    #[arg(long)]
    pub show_passwords: bool,
}

#[derive(Debug, Args)]
pub struct CreateUserArgs {
    /// E-mail address of the user to create; the account is created with this
    /// address, even when the alias file knows other ones for the same person
    pub email: String,

    #[command(flatten)]
    pub create: CreateArgs,
}

#[derive(Debug, Args)]
pub struct DeleteArgs {
    /// Re-assign the content of deleted users to this user ID; without it,
    /// their content is deleted too
    #[arg(long, env = "WP_REASSIGN_TO")]
    pub reassign: Option<u64>,
}

#[derive(Debug, Args)]
pub struct SyncArgs {
    #[command(flatten)]
    pub create: CreateArgs,

    #[command(flatten)]
    pub delete: DeleteArgs,
}
