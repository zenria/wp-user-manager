# CLAUDE.md

Guidance for Claude Code when working in this repository.

## What this is

`wp-user-manager` is a Rust CLI that reconciles the users of a WordPress site
with a reference file listing e-mail addresses. It reports differences and,
after an interactive confirmation, creates missing users and deletes surplus
ones. Language of the code, comments, CLI output and documentation: **English**.

## Commands

```sh
cargo build                       # build
cargo test                        # unit tests (all inline `mod tests`)
cargo clippy --all-targets        # must stay warning-free
cargo fmt                         # rustfmt defaults
cargo run -- status               # run a subcommand
```

There is no CI config yet; before finishing a change, run `cargo fmt`,
`cargo clippy --all-targets` and `cargo test`.

## Architecture

One module per concern, all in `src/`, no library crate:

- `main.rs` — loads `.env` (`dotenvy`) **before** `Cli::parse()`, since clap
  resolves options from the environment; dispatches to `commands`.
- `cli.rs` — the whole clap surface. Global options are `#[arg(global = true)]`
  with an `env = "WP_…"` fallback, so every option is also configurable via
  `.env`. Subcommand-specific options live in `CreateArgs` / `DeleteArgs`.
- `reference.rs` — parses the reference file into `Identity` values. One line =
  one person, possibly several e-mails (separators: whitespace, `,`, `;`;
  `#` starts a comment). `Email` keeps the address as written (`raw()`) *and*
  its comparison key (`key()`), produced by `Normalizer` (lowercase, plus
  optional `+tag` stripping). Duplicate addresses across two lines are a hard
  error: they would make matching ambiguous.
- `reconcile.rs` — pure matching logic, no I/O. `reconcile()` returns a `Plan`
  with `matched`, `ambiguous`, `missing`, `extra` and `protected`. This is
  where the invariants live; it is the most valuable place for tests.
- `provision.rs` — derives username, display name and a random password for a
  user to be created; keeps usernames unique against those already taken.
- `wp.rs` — WordPress REST client (`/wp-json/wp/v2`). Basic auth with an
  application password (spaces stripped). `WpError` (thiserror) distinguishes
  transport, API (`code`/`message` from the JSON body) and 401.
- `prompt.rs` — the confirmations. `confirm()` for yes/no, `confirm_phrase()`
  for destructive actions. Both refuse to guess when stdin is not a terminal.
- `report.rs` — all the plain-text output formatting.
- `commands.rs` — one function per subcommand. `open_session()` is the shared
  entry point: it loads the reference file, builds the client, identifies the
  authenticated user, lists the WordPress users and reconciles.

Error handling: `anyhow` at the command/`main` level (with `.context()` for
the operation that failed), `thiserror` for the domain errors of `wp.rs` and
`reference.rs`.

## Rules to keep

- **No write without confirmation.** Any new modifying command goes through
  `prompt::confirm*` and honours the global `--yes`. Never bypass the prompt
  because a run looks non-interactive: abort instead.
- **Safety nets in `reconcile.rs`**, not in the commands. The authenticated
  user is always protected (`open_session`), administrators are protected
  unless `--delete-administrators` is passed, and `--protect` adds addresses.
- **Ambiguous identities are never acted upon**, only reported.
- E-mails are compared through `Normalizer::normalize`, never with `==` on raw
  strings. Send the raw address to WordPress, compare on the key.
- Every new behaviour of `reference.rs`, `reconcile.rs` or `provision.rs` gets
  a unit test in the module's `mod tests`. `wp.rs` and `commands.rs` do I/O
  and are covered manually (see below).
- Adding a global option means: a field in `GlobalArgs` with its `env`, a line
  in `.env.example`, and a row in the README table if it is a command.

## Manual testing against a mock WordPress

There is no integration test harness. To exercise the HTTP paths without a
real site, run a small mock REST server (`users/me`, `GET/POST/DELETE
wp/v2/users`, honouring `context=edit`, `per_page`/`page` with an
`X-WP-TotalPages` header, and `force=true`/`reassign` on delete), then point
the CLI at it with `WP_URL=http://127.0.0.1:<port>`. Worth checking:
pagination beyond 100 users, alias matching, 401 mapping, the `delete`
confirmation phrase, and the non-interactive abort.

## WordPress API notes

- Listing users requires `context=edit`, otherwise e-mails are absent from the
  response.
- Pagination: `per_page` is capped at 100 by WordPress; follow the
  `X-WP-TotalPages` response header.
- `DELETE /users/{id}` **requires** `force=true` *and* a `reassign` parameter;
  we send `reassign=false` when no target user is given, which deletes the
  user's content as well.
- Creating a user requires `username`, `email` and `password`. WordPress
  rejects e-mails already in use with the `existing_user_email` code.

## Not to be done

- Do not log or print application passwords, and do not print generated
  passwords unless `--show-passwords` is set.
- Do not commit a `.env` file (it is git-ignored); keep `.env.example` up to
  date instead.
- Do not put real user e-mail addresses in tests, fixtures or commit messages:
  use `example.com` addresses.
