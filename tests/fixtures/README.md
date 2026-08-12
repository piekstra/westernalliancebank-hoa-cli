# Fixtures

Scrubbed captures of real portal responses, used to test the parsers offline.

## What they are

Each file is a fragment of an actual response — the part `wabhoa` reads —
captured from `pay.westernalliancebank.com` on 2026-08-06:

| File | Source | Parsed by |
| --- | --- | --- |
| `make_payment_selects.html` | `GET /Payment/MakePayment` | `parse::properties`, `parse::payment_methods` |
| `dashboard_scheduled.html` | `GET /DashboardContent` | `parse::scheduled_payments` |
| `payment_history_page.html` | `GET /Payment/PaymentHistory` | `parse::site_user_id` |
| `payment_history_search.json` | `POST /Payment/PaymentHistorySearch` | `parse::payments` |
| `payment_options.json` | `POST /Homeowner/PreSelectedPaymentOptions` | `parse::balance` |
| `notifications.html` | `GET /Notifications/List` | `parse::notifications` |
| `profile.html` | `GET /Account/Profile` | `parse::profile` |
| `statement_history_published.html` | `GET /Properties/StatementHistory` | `parse::statements` (populated case; the test account has none, so the row shape is reconstructed from the portal's `DownloadStatement(fileName, fileAlias)` handler) |

## The scrubbing policy

**Structure is preserved exactly; every identifying or financial value is
replaced with an obvious dummy.** Attribute names, nesting, key order, date
encodings, and the portal's quirks (its `/Date(…)/` epochs, its `Submitt`
typo, blank-vs-absent fields) all survive — those are what the tests are
about. What must not survive:

- Names, email addresses, phone numbers, street addresses
- Property IDs, account numbers, association codes, management company IDs
- Transaction numbers, notification IDs, payment method IDs, session cookies
- Real dollar amounts
- **URL tokens.** Notification bodies embed
  `/Account/ExpressLogin?token=<uuid>` and
  `/payment/CancelPayment?token=<uuid>` links. An ExpressLogin token is a
  *working credential* — anyone holding it can sign into the account. Replace
  every UUID with a `00000000-0000-4000-8000-…` dummy; the test rejects any
  UUID that doesn't start `00000000-`.

That last one is not hypothetical: gitleaks caught two live tokens in this
directory before the first push. A value can look harmless in a rendered email
and still be a credential.

Replacements are deliberately unrealistic (`1 Sample St`, `Sample Owner`,
`SAMPLE BANK, NA`, `100.00`) so a reader can tell at a glance that a value is
fake. `fixtures_carry_no_real_identifiers` in `../fixture_shapes.rs` enforces
this with a banned-substring list; **extend that list whenever you add a new
kind of identifier.**

The banned list holds patterns, not the real values — a denylist of actual
account numbers would itself be the leak.

## Refreshing a fixture

1. Capture the response (`wabhoa api <path> --raw`, or the browser's network
   tab).
2. Cut it down to the fragment the parser actually reads.
3. Apply the scrub map above — consistently, so cross-references still line up
   (the same property must keep the same fake ID in every file).
4. Run `cargo test`; the shape tests will tell you what drifted.

Never commit a raw capture "just to scrub it later."
