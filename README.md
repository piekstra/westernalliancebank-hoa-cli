# wabhoa — Western Alliance Bank HOA payment portal CLI

[![CI](https://github.com/piekstra/westernalliancebank-hoa-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/piekstra/westernalliancebank-hoa-cli/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/piekstra/westernalliancebank-hoa-cli?sort=semver)](https://github.com/piekstra/westernalliancebank-hoa-cli/releases)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

Read your HOA assessment account from the command line: properties and
balances, payment history, recurring schedules, saved payment methods, and the
notices the portal emails you — from the Western Alliance Bank community
association payment portal your management company uses. Fast, keychain-secured,
agent-friendly.

**Every command is a read.** Nothing in this CLI moves money, changes a payment
schedule, or edits a saved account. Run `wabhoa writes` to see exactly what the
portal *can* do that this CLI deliberately doesn't.

> Unofficial and not affiliated with Western Alliance Bank. It drives the
> portal's own undocumented endpoints, which can change without notice. See
> [`docs/api.md`](docs/api.md).

## Install

```console
# from source (recent stable Rust; MSRV 1.88)
$ cargo install --git https://github.com/piekstra/westernalliancebank-hoa-cli --locked
$ make install        # cargo install --path . --force
```

Prebuilt tarballs ship with each release; `wabhoa self-update` pulls the latest.

## Quick start

```console
$ wabhoa config set username you@example.com
$ wabhoa auth login          # prompts for your portal password
$ wabhoa summary
```

`auth login` stores your password in the OS keychain and caches the portal
session there too. Later commands reuse the session; when the portal expires it,
any command exits 3 and tells you to run `auth login` again.

To load the password from a password manager instead of typing it:

```console
$ op read 'op://Private/pay.westernalliancebank.com/password' \
    | wabhoa auth set-credential --stdin --overwrite
```

There is no two-factor step — the portal doesn't have one.

### Other banks on the same platform

The portal is white-labeled. Alliance Association Bank runs the same
application, so pointing `base_url` at it is expected to work:

```console
$ wabhoa config set base_url https://pay.allianceassociationbank.com
```

## Commands

```console
wabhoa summary                        # balances, schedules, recent payments
wabhoa properties list                # properties, with association + account IDs
wabhoa properties get <id>            # one property, incl. published balance
wabhoa payments list                  # payment history, newest first
wabhoa payments list --start 2026-01-01 --end 2026-06-30
wabhoa payments list --status Processed --limit 100
wabhoa payments get <transaction#>    # one payment in full
wabhoa scheduled list                 # recurring payments the portal will make
wabhoa methods list                   # saved bank accounts/cards, masked
wabhoa notifications list             # payment notices sent to you
wabhoa notifications get <id>         # one notice, with its message body
wabhoa statements list                # statement packets, if published
wabhoa profile                        # the account holder on file
wabhoa writes                         # portal writes this CLI does NOT do
wabhoa api <path> [--data JSON] [--raw]   # raw passthrough
wabhoa auth login | status | logout | set-credential
wabhoa config path | show | set <k> <v> | unset <k>
wabhoa self-update [--check] [-y]
wabhoa completions <shell>
wabhoa info                           # machine-readable discovery (cli-info/v1)
```

Every command takes `--json`. `list` subcommands alias to `ls`.

## Examples

```console
$ wabhoa summary
Properties:
ADDRESS | ASSOCIATION_ID | ACCOUNT_NUMBER | BALANCE_PUBLISHED
1 Sample St | SA | 222222 | false

Scheduled payments:
PROPERTY | NEXT_PAYMENT_DATE | FREQUENCY | AMOUNT
1 Sample St | 2026-09-01 | Monthly | 100.0

Recent payments:
PAYMENT_DATE | PROPERTY | AMOUNT | STATUS
2026-08-01 | 1 Sample St | 100.0 | Processed
2026-07-01 | 1 Sample St | 100.0 | Processed
```

```console
$ wabhoa payments list --limit 2
PAYMENT_DATE | PROPERTY | AMOUNT | STATUS | TYPE | TRANSACTION_NUMBER
2026-08-01 | 1 Sample St | 100.0 | Processed | ECheck | 10000001
2026-07-01 | 1 Sample St | 100.0 | Processed | ECheck | 10000002
```

```console
$ wabhoa --json payments get 10000001
{
  "schema": "payment/v1",
  "transaction_number": "10000001",
  "payment_date": "2026-08-01",
  "processed_date": "2026-08-02",
  "status": "Processed",
  "type": "ECheck",
  "property": "1 Sample St",
  "payment_method": "SAMPLE BANK, NA X-0000",
  "amount": 100.0,
  "fee": 0.0,
  "total": 100.0
}
```

### About balances

Many associations don't publish a balance through this portal. When that's the
case the portal reports `0`, which reads as "paid up" but means "not published".
`wabhoa` reports `balance_published: false` and **omits** the balance instead:

```console
$ wabhoa properties get 1111111
...
balance_published: false
```

If your association does publish one, `balance` and `next_assessment_date`
appear alongside it.

## Configuration

`~/.config/wabhoa/config.json`, via `wabhoa config`:

| Key | Default | Meaning |
| --- | --- | --- |
| `username` | — | Portal login email. Also read from `$WABHOA_USERNAME`. |
| `base_url` | `https://pay.westernalliancebank.com` | For the other banks on this platform. |

Secrets never land here. The password and the cached session live in the OS
keychain under service `piekstra.wabhoa`, accounts `password` and `session`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Keychain or unclassified failure |
| 2 | Usage error |
| 3 | Auth required or session expired — run `wabhoa auth login` |
| 4 | Not found |
| 5 | Portal/network failure |
| 6 | Refused: a write was requested of a read-only CLI |

## Development

```console
$ make verify     # fmt-check + clippy -D warnings + tests + smoke — the CI gate
$ make test       # unit + integration; fully offline, no network or keychain
$ make dev        # debug build, re-signed so keychain grants survive rebuilds
```

Tests run the real parsers over scrubbed captures of real portal responses; see
[`tests/fixtures/README.md`](tests/fixtures/README.md) before touching one.
[`AGENTS.md`](AGENTS.md) has the conventions and the portal's gotchas.

## License

MIT OR Apache-2.0.
