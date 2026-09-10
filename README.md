# wp-user-manager

A CLI tool that reconciles the users of a WordPress site with a reference list
of e-mail addresses. It can report what is missing, what is in excess, create
the missing accounts and delete the surplus ones. Every modifying action asks
for an explicit confirmation.

## Install

```sh
cargo build --release
# binary in ./target/release/wp-user-manager
```

## Configure

Every option can be given on the command line or through an environment
variable, which may live in a `.env` file next to the binary (see
`.env.example`):

```sh
cp .env.example .env
```

Authentication uses a WordPress **application password**, created in
*Users > Profile > Application Passwords*. The account needs the `list_users`,
`create_users` and `delete_users` capabilities — in practice, an administrator.

## The reference file

One *identity* per line. A line may hold several addresses (separated by
whitespace, `,` or `;`) that all designate the same person: this is how a
WordPress account registered under an old address is reconciled with the new
one. The first address of a line is the primary one, used at creation time.
Blank lines and `#` comments are ignored.

```
alice@example.com
bob@example.com, bob.smith@old-company.com
```

Here, a WordPress user whose e-mail is `bob.smith@old-company.com` is
considered present: nothing is created and nothing is deleted.

## Commands

| Command | Effect |
| --- | --- |
| `list-reference` | Shows the identities read from the file (no WordPress call) |
| `list-users` | Lists the WordPress users |
| `status` | Full reconciliation report |
| `list-missing` | Identities with no WordPress account |
| `list-extra` | WordPress users absent from the reference file |
| `create-missing` | Creates the missing users, after confirmation |
| `delete-extra` | Deletes the surplus users, after confirmation |
| `sync` | `create-missing` then `delete-extra`, each confirmed separately |

```sh
wp-user-manager status
wp-user-manager create-missing --role subscriber
wp-user-manager delete-extra --reassign 1
```

## Safety

- Read-only commands never modify anything; `create-missing`, `delete-extra`
  and `sync` prompt before writing, and `delete-extra` requires the word
  `delete` to be typed.
- The account used to authenticate is never deleted.
- Administrators absent from the reference file are kept, unless
  `--delete-administrators` is passed.
- `--protect a@b.com,c@d.com` shields extra addresses from deletion.
- Deletion is permanent (WordPress `force=true`); `--reassign <ID>` transfers
  the content of the deleted users, otherwise it is deleted with them.
- Created users get a random password that is not displayed (unless
  `--show-passwords`): send them through the "lost password" flow.
- When an identity matches *several* WordPress accounts, the line is reported
  as ambiguous and left untouched.
- `--yes` answers every prompt, for non-interactive runs. Without it, and
  without a terminal, modifying commands abort.
