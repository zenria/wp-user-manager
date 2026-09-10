mod aliases;
mod cli;
mod commands;
mod prompt;
mod provision;
mod reconcile;
mod reference;
mod report;
mod wp;

use clap::Parser;

use cli::{Cli, Command};

#[tokio::main]
async fn main() {
    // Options are resolved from the environment by clap, so the `.env` file
    // has to be loaded first. A missing file is not an error.
    let _ = dotenvy::dotenv();

    let cli = Cli::parse();
    let result = run(&cli).await;

    if let Err(error) = result {
        eprintln!("\nError: {error:#}");
        std::process::exit(1);
    }
}

async fn run(cli: &Cli) -> anyhow::Result<()> {
    let global = &cli.global;
    match &cli.command {
        Command::ListReference => commands::list_reference(global),
        Command::ListAliases => commands::list_aliases(global),
        Command::ListUsers => commands::list_users(global).await,
        Command::Status => commands::status(global).await,
        Command::ListMissing => commands::list_missing(global).await,
        Command::ListExtra => commands::list_extra(global).await,
        Command::CreateMissing(args) => commands::create_missing(global, args).await,
        Command::DeleteExtra(args) => commands::delete_extra(global, args).await,
        Command::Sync(args) => commands::sync(global, &args.create, &args.delete).await,
    }
}
