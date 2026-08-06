# AGENTS.md

Guidance for AI coding agents (and humans) working in this repo. Tool-agnostic;
`CLAUDE.md` points here.

## What this is

`wabhoa` — a Rust CLI for the **Western Alliance Bank community association
payment portal** (`pay.westernalliancebank.com`), the site management companies
hand to homeowners for paying HOA assessments. A thin, portal-specific layer
over the shared [`cli-common`](https://github.com/piekstra/cli-common)
`pk-cli-*` crates (auth, http, config, secrets, self-update). This repo owns the
portal client, the parsers, the commands, and their DTOs.

There is no official API. Everything targets the undocumented endpoints the
portal's own front end calls, mapped by watching its traffic and reading its
`starscream.*` JavaScript, and written up in [`docs/api.md`](docs/api.md).

## Build, test, lint

```console
make verify     # fmt-check + clippy -D warnings + tests + smoke — the CI gate
make test       # unit + integration (fully offline; no network, no creds)
make dev        # debug build, re-signed so keychain grants survive rebuilds
cargo run -- summary
```

Run `make verify` before considering a change done — it's exactly what CI runs.

## Layout

- `src/lib.rs` — module root. Modules are public so integration tests can drive
  the parsers directly; `main.rs` is a thin clap shell over them.
- `src/main.rs` — command tree, login flow, exit-code mapping.
- `src/client.rs` — the HTTP client: cookie session, login, rotation write-back,
  and the redirect-means-expired detection. Its module doc explains the auth
  model.
- `src/html.rs` — small, total HTML scanners. No DOM parser dependency on
  purpose; see its module doc.
- `src/parse.rs` — page/JSON → flat DTOs. **The interesting logic lives here**,
  because the portal has no API contract, only markup that can drift.
- `src/commands/*.rs` — one module per command group; each renders a human table
  and a `--json` DTO.
- `src/writes.rs` — catalog of the portal's mutating endpoints. Powers both
  `wabhoa writes` and the `api` POST guard.
- `src/config.rs` — non-secret config; every secret is keychain-only.
- `src/dates.rs` — the portal's three date dialects ⇄ ISO.
- `tests/` — offline surface + contract tests, and `tests/fixtures/` (read its
  README before touching a fixture).

## Conventions (do not break these)

- **`--json` on every command**, emitting one DTO tagged with a `schema` field
  (e.g. `"schema":"payment-list/v1"`). Human output → stdout as a table;
  diagnostics → stderr. Keep the two paths in sync; a breaking DTO change bumps
  the `/vN` suffix.
- **Exit codes:** 0 ok · 2 usage · 3 auth · 4 not found · 5 upstream · 6
  refused-write. Validate args **before** touching the keychain or network, so
  `--help` and bad args never prompt, hang, or hit the portal. `payments list`
  resolves its date range twice for exactly this reason — don't "fix" it.
- **Read-only.** Every command observes. If a write is ever added it must prompt
  for confirmation, require `--force` non-interactively (exit 6 otherwise), be
  removed from `src/writes.rs`, and this section must stop saying "read-only" —
  as must the README's second paragraph and `writes.rs`'s module doc.
- **Secrets** come from the OS keychain or stdin — never argv, never logs, never
  a file in the repo. Service `piekstra.wabhoa`, accounts `password`, `session`.
- **Dates** are ISO `YYYY-MM-DD` at the CLI boundary, converted in `dates.rs`.
  Never surface the portal's `MM/DD/YYYY` or `/Date(…)/` in a flag or a DTO.
- **Parsers must be total.** A portal redesign should empty a table, not panic.
  `parsers_tolerate_unrecognized_markup` enforces it; keep new parsers in it.

## Portal-specific gotchas

- **A zero balance usually means "not published", not "paid up".** When
  `ShowDisplayBalance` is `false` the portal reports `0` for a balance it simply
  doesn't have. `parse::balance` omits the field and sets
  `balance_published: false`; `parse::properties` drops `attr_Balance` for the
  same reason. Do not "restore the missing balance column" — printing `0.00`
  there tells the user they owe nothing, which may be false.
- **An expired session 302s to `/Home`, not 401.** The client treats landing on
  `/Home`, an HTML body where JSON was expected, and a `200` containing
  `id="txtPassword"` as `CliError::Auth`. Don't reduce that to a status check.
- **`PaymentTotal` is `0` on processed records.** The portal computes it only
  while composing a payment. `total` is derived as amount + fee.
- **.NET sentinel dates.** `/Date(-62135596800000…)/` is `DateTime.MinValue` and
  `01/01/1900` is the "no assessment scheduled" placeholder. Both normalize to
  absent. A year-0001 date in output is a bug.
- **The portal's own typo is load-bearing:** the payment endpoint is
  `/Payment/SubmittPayment`, two `t`s. Don't correct it in `writes.rs` — the
  guard matches on the real path.
- **Cloudflare may obfuscate email cells** into `data-cfemail` hex. Requests
  with `X-Requested-With: XMLHttpRequest` seem to get plain addresses. Both
  parse — and `data-cfemail` must never reach a fixture (it decodes to the real
  address; the banned-substring test covers it).
- **The transaction-detail modal is client-side.** There is no per-transaction
  endpoint; `payments get` filters the search response.
- **`/Payment/MakePayment` is a GET and reading it pays nothing.** It's the only
  page exposing each property's `MgCoId`/`AssocId`/`MemberId` triple, which the
  balance endpoint is keyed on. Don't move properties to `/Properties/Manage`,
  which omits them.

## Safety & privacy (written as if this repo were public)

- Never commit a password, session cookie, real name, address, balance,
  transaction number, or account/property ID.
- `tests/fixtures/` are **scrubbed** captures: structure preserved exactly,
  every identifying and financial value replaced with an obvious dummy. The
  policy is in `tests/fixtures/README.md`, and
  `fixtures_carry_no_real_identifiers` in `tests/fixture_shapes.rs` enforces it.
  Extend that test's banned list when you add a new kind of identifier.
- Don't paste real portal output into an issue, commit message, or doc example.
  The README's examples use scrubbed figures.

## Definition of done

`make verify` green, tests cover the change, `--json` and human output both
updated, `docs/api.md` still matches reality, and no secrets or PII anywhere in
the diff — including fixtures.
