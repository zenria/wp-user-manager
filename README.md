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

One address per line, one line per user — this file, and only this file, says
who must exist. Blank lines and `#` comments are ignored. A line holding
several addresses is rejected: alternative addresses go in the alias file.

```
alice@example.com
bob@example.com
```

## The alias file (optional)

A pure mapping: one group per line, at least two addresses (separated by
whitespace, `,` or `;`) that designate the same person.

```
bob@example.com, bob.smith@old-company.com
carol@example.com carol.doe@example.org
```

Listing an address here **never** makes it a user to create; it only tells the
tool which WordPress accounts are the same person. So with the reference file
above, a WordPress user whose e-mail is `bob.smith@old-company.com` counts as
present: nothing is created, nothing is deleted. And `carol.doe@example.org`
alone is not a reference entry — Carol is expected because
`carol@example.com` is in the reference file.

Order carries no meaning, on either axis: the addresses on a line are a set,
and two lines sharing an address are merged into one group. Reconciliation
therefore works in both directions — it does not matter whether the reference
file lists the new or the old address.

## Commands

| Command | Effect |
| --- | --- |
| `list-reference` | Shows the identities read from the reference file (no WordPress call) |
| `list-aliases` | Shows the alias groups, after merging (no WordPress call) |
| `list-users` | Lists the WordPress users |
| `status` | Full reconciliation report |
| `list-missing` | Identities with no WordPress account |
| `list-extra` | WordPress users absent from the reference file |
| `create-missing` | Creates the missing users, after confirmation |
| `create-user <EMAIL>` | Creates a single user from one address, after confirmation |
| `delete-extra` | Deletes the surplus users, after confirmation |
| `sync` | `create-missing` then `delete-extra`, each confirmed separately |

```sh
wp-user-manager list-aliases
wp-user-manager status
wp-user-manager create-missing --role subscriber
wp-user-manager create-user jane.doe@example.com --role subscriber
wp-user-manager delete-extra --reassign 1
```

`create-user` does not need a reference file: the address on the command line
says who to create. It still resolves the address through the alias file, so
an account already registered under another address of the same person is
found and nothing is created. When a reference file *is* configured and does
not list the address, the command warns: the reference file remains the source
of truth, and `delete-extra` would otherwise propose the new account for
deletion.

## Safety

- Read-only commands never modify anything; `create-missing`, `create-user`,
  `delete-extra` and `sync` prompt before writing, and `delete-extra` requires the word
  `delete` to be typed.
- The account used to authenticate is never deleted.
- Administrators absent from the reference file are kept, unless
  `--delete-administrators` is passed.
- `--protect a@b.com,c@d.com` shields extra addresses from deletion, aliases
  included.
- Deletion is permanent (WordPress `force=true`); `--reassign <ID>` transfers
  the content of the deleted users, otherwise it is deleted with them.
- Created users get a random password that is not displayed (unless
  `--show-passwords`): send them through the "lost password" flow.
- When an identity matches *several* WordPress accounts (typically two
  accounts opened under two addresses of the same alias group), it is reported
  as ambiguous and left untouched.
- A person listed twice in the reference file, directly or through an alias, is
  a hard error rather than a silent duplicate.
- `--yes` answers every prompt, for non-interactive runs. Without it, and
  without a terminal, modifying commands abort.
