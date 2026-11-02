# Western Alliance Bank HOA payment portal — observed API

Reverse-engineered from the portal's own traffic at
`https://pay.westernalliancebank.com` on **2026-08-06** (statement-download
endpoint added **2026-08-11**). Unofficial and undocumented: Western Alliance
Bank publishes no API for this portal, and any of this can change without
notice. `wabhoa api <path> --raw` is the escape hatch for checking.

The application is **ServiceStack on ASP.NET**, fronted by Cloudflare, and is
white-labeled per management company (the same deployment serves Alliance
Association Bank at `pay.allianceassociationbank.com` — hence the `base_url`
setting). The portal's front-end JavaScript is namespaced `starscream.*`, which
is where most of the endpoint map below came from.

## Authentication

Auth is **cookie-only**. There is no bearer token, API key, CSRF token on
reads, or refresh flow — and **no second factor**.

| Step | Request | Notes |
| --- | --- | --- |
| 1 | `POST /auth/credentials` | JSON `{"UserName": "…", "Password": "…", "RememberMe": false}`. |
| 2 | → `200` | Sets `ss-id`, `ss-pid`, `ss-opt` (ServiceStack session) and `opayUC` (portal user context). |
| 2′ | → `401` | `{"ResponseStatus":{"ErrorCode":"Unauthorized","Message":"Invalid Username or Password"}}`. |

Those cookies authenticate every later read on their own. `wabhoa` caches the
bundle in the OS keychain as a single `name=value; …` string under account
`session`.

A missing password is rejected by FluentValidation *before* the credential
check, with a `ValidationException` naming the field — useful for telling
"empty keychain entry" apart from "wrong password".

### No 2FA, and no bot wall on login

Unlike most portals in this family, `POST /auth/credentials` works from a plain
HTTP client with no browser involvement: no captcha, no device fingerprinting,
no SMS code. Cloudflare's `__cf_bm` cookie is issued but not required. A recent
desktop Chrome `User-Agent` is set anyway, since the edge does reject obviously
scripted clients.

This is why `wabhoa` uses the `password` auth method rather than the
`browser-session` pattern that Xfinity needs.

### Session expiry

The portal **never answers `401`** for an expired session. It refuses in three
different ways depending on the endpoint, all confirmed against a genuinely
expired session on 2026-08-07, and all mapped to `CliError::Auth` (exit 3):

| Tell | Endpoints observed | Shape |
| --- | --- | --- |
| Bounced to the login page | `/Payment/MakePayment`, `/DashboardContent` | 302 → `/Home` |
| Bounced, path preserved | `/Properties/StatementHistory` | 302 → **`/Home/Properties/StatementHistory`** |
| Empty page | `/Account/Profile`, `/Notifications/List` | **HTTP 200, `Content-Length: 0`, no redirect** |

Two traps here, both of which this client got wrong at first:

- The redirect **prefixes** the requested path onto `/Home`, so a check for a
  URL *ending* in `/Home` catches only the bare case. Match on the path
  starting with `/Home`.
- The empty-200 case has no redirect and no marker of any kind. A parser fed a
  zero-byte page just returns nothing, so the CLI reported "no notifications
  found" and exit 0 to a logged-out user — a wrong answer, not an error.
  Nothing this CLI requests legitimately answers with an empty body, so an
  empty body is always expiry.

An HTML body where JSON was expected is treated the same way.

### Session rotation

ServiceStack re-issues session cookies at its discretion. `wabhoa` writes the
rotated bundle back to the keychain after every successful read; a client that
kept replaying its original cookies would eventually be logged out despite the
portal having just handed it live ones.

## Reads

Only two reads answer in JSON. The rest are Razor views, so the data is scraped
out of the markup — see `src/parse.rs`.

| Endpoint | Method | Answers | Carries |
| --- | --- | --- | --- |
| `/Payment/MakePayment` | GET | HTML | **Properties and payment methods.** Two `<select>`s: `idPropertyMember` and `idPaymentMethod`. |
| `/Payment/PaymentHistory` | GET | HTML | Hidden `idSiteUserLogin` — required by the history search. |
| `/Payment/PaymentHistorySearch` | POST | **JSON** | Full payment history. |
| `/Homeowner/PreSelectedPaymentOptions` | POST | **JSON** | Balance, next assessment date, management company. |
| `/DashboardContent` | GET | HTML | Scheduled (recurring) payments; recent payments. |
| `/Notifications/List` | GET | HTML | Notification rows + bodies in a `messagesArray` literal. |
| `/Properties/StatementHistory` | GET | HTML | Statement packets, when the association publishes any. |
| `/Statements/GetStatementByteArray` | POST | **JSON** | One statement's PDF bytes, as base64. Body: `{"FileName": "<file>"}`. |
| `/Account/Profile` | GET | HTML | Name, plus **masked** phone and email. |
| `/PaymentMethods/Manage` | GET | HTML | Payment methods again, but without their IDs — `MakePayment` is the better source. |

### Shapes worth knowing

**`POST /Payment/PaymentHistorySearch`** — every field is required by the model
binder; omitting one is a validation error, not a wildcard. Unused filters go
as empty strings, but the date bounds want `null`:

```json
{"PropertyId":"","PaymentDate":null,"PaymentDateMin":null,"PaymentDateMax":null,
 "PaymentAmount":"","PaymentAmountMin":"","PaymentAmountMax":"",
 "PaymentStatus":"","TransactionNumber":"","SiteUserLogin":"<id>"}
```

Answers `{"HistoryPaymentsByUserList": [...], "SiteContext": {...}, …}`. Records
carry `PaymentDate`, `ProcessDate`, `TransactionNumber`, `PaymentAmount`,
`PaymentFee`, `Status`, `PaymentType`, `PropAddress`, `PaymentMoniker`,
`MngmtCoId`, `AssocId`, `MemberId`.

Dates are ASP.NET epochs — `"/Date(1785567600000-0700)/"`. The trailing offset
is the *server's* timezone rendering of an instant the millis already express in
UTC, so it is ignored; applying it shifts evening timestamps back a day.

`PaymentTotal` is `0` on processed records — the portal only computes it while a
payment is being composed — so `wabhoa` derives `total` as amount + fee.

The transaction-detail modal is rendered **client-side from this same
response**; there is no per-transaction endpoint.

**`POST /Homeowner/PreSelectedPaymentOptions`** — keyed on the property's
identity triple, which only the `MakePayment` select exposes:

```json
{"ManagementCompanyId":"<MgCoId>","AssociationId":"<AssocId>","PropertyAccountNumber":"<MemberId>"}
```

`ShowDisplayBalance` is the field that matters: when it is `false` the
association publishes no balance and `TotalNewBalanceAmountDue` is `0` —
which means *unknown*, not *paid up*. `wabhoa` reports `balance_published:
false` and omits the balance rather than printing a misleading `0.00`.

**Property `<option>` attributes** (`idPropertyMember`) — the only place the
full identity appears:

```html
<option value="<memberPropertyId>" attr_Balance="0" attr_Address="…"
        attr_MgCoId="…" attr_AssocId="…" attr_MemberId="…" attr_AltMemberId="…"
        attr_Fee="0" attr_DebitFee="0" attr_StopCode="" attr_hasamenities="no">
```

`attr_Balance` has the same "0 means unknown" problem as above and is
deliberately not surfaced.

**Scheduled payments** (`/DashboardContent`) — clean `data-*` attributes on the
row, so no `onclick` parsing is needed:

```html
<tr id="ECheck<id>" paymentType="ECheck" data-payment-id="<id>"
    data-frequency="Monthly" data-next-date="09/01/2026" data-end-date=""
    data-amount="100.0000" data-fee="0.0000">
```

**Notifications** — the table holds date/subject/to/cc; the message bodies are
pushed into a JavaScript literal on the same page:

```js
messagesArray.push({ Id: 50000001, Message: '<p>Dear …</p>' });
```

`wabhoa` zips the two by row order. Cloudflare sometimes obfuscates the "To"
cell into `<a class="__cf_email__" data-cfemail="…">[email protected]</a>`;
requests carrying `X-Requested-With: XMLHttpRequest` appear to get the plain
address instead. Both forms parse.

**Notification bodies are prose, not attachments.** Every message observed on
the test account is an HTML `<p>`-and-`<table>` payment notice — receipts,
upcoming-payment reminders — with no PDF file, no `<img data:…>`, no `<a
href="…pdf">`. The only URLs embedded are one-time-use `/Account/ExpressLogin`
and `/payment/CancelPayment` tokens; those are working *credentials* rather
than attachments, so `notifications get` deliberately does not surface them
as a downloadable side-channel. Statement PDFs are on the statement-history
surface (below), not attached to a notification.

**`POST /Statements/GetStatementByteArray`** — the portal's own front end calls
this from `DownloadStatement(fileName, fileAlias)` on `StatementHistory.cshtml`.

```json
{"FileName": "<opaque server key>"}
```

Answers a JSON envelope with the PDF as base64 text — not a raw binary stream:

```json
{"IsSuccessful": true, "File": "<base64 bytes>"}
```

On failure, `IsSuccessful` is `false` and `StatusMessage` carries a human hint.
The `FileName` comes off the statement-history table row's
`onclick="DownloadStatement('…','…')"`; there is no separate list endpoint.
This is the read `wabhoa documents download` uses.

**The failure is an HTTP 200.** Confirmed live on 2026-08-11 against a
`FileName` that does not exist — the endpoint answers `200 OK` with

```json
{"IsSuccessful": false, "StatusCode": -1100,
 "StatusMessage": "There was an error trying to perform the requested action. …"}
```

and **no `File` key at all** — not `null`, absent. A client that trusts the
status line writes an empty file and calls it a statement.
`client::statement_error` requires `IsSuccessful: true` *and* a `File` string
before anything is decoded.

**Confirm the bytes are a PDF; the envelope is not enough.** `File` is base64
text, so whatever the portal puts there arrives looking like a successful
download — including an HTML error or login page. Nothing else distinguishes
the two: the status line is `200`, the envelope says success, and the
`Content-Type` is `application/json` either way, so there is no content-type
signal to lean on. The only positive signal is the payload itself.
`client::pdf_error` therefore requires a `%PDF-` header within the first
kilobyte of the decoded bytes and fails with exit **5** otherwise, calling out
HTML specifically as a probably-lapsed session. Nothing reaches disk unless
that check passes.

**The envelope carries its own auth state.** Every response nests
`UserContext.AuthenticationState` (`"Authenticated"` on a live session)
alongside `SiteContext`. Worth knowing as a cross-check if the expiry shapes
above ever stop being reliable on this endpoint.

**A `)` in a description is not the end of the call.** Statement descriptions
are free text and routinely parenthesized — `DownloadStatement('9001.pdf',
'March 2026 Statement (archived)')`. Scanning for the call's closing `)`
without tracking quotes stops inside `(archived)` and truncates the alias.
That corrupts the listed description *and* the name the PDF is saved under,
silently and with a zero exit. `parse::closing_paren` is quote-aware for this
reason.

### Date dialects

Three, all normalized to ISO `YYYY-MM-DD` at the CLI boundary:

| Where | Format | Sentinel for "unset" |
| --- | --- | --- |
| JSON responses | `/Date(1785567600000-0700)/` | `-62135596800000` (.NET `DateTime.MinValue`, year 1) |
| HTML and search filters | `MM/DD/YYYY` | `01/01/1900` |
| Payment options | `/Date(-2208960000000-0800)/` | that *is* 1900-01-01 |

Both sentinels are normalized to absent — surfacing a year-0001 date would be
worse than saying nothing.

## Writes — catalogued, not implemented

`wabhoa` is read-only. The portal's mutating endpoints are transcribed in
`src/writes.rs` and printed by `wabhoa writes`; that catalog is also what makes
`wabhoa api` refuse to POST to them (exit 6).

**None of them have been called.** The paths come from reading the portal's
JavaScript, so payload shapes are unverified — anything implementing them must
confirm the contract first, against an account where a mistake is affordable.

Highest-stakes group, for orientation:

| Endpoint | Effect |
| --- | --- |
| `POST /Payment/SubmittPayment` | Submits an eCheck payment. (The double `t` is the portal's.) |
| `POST /Payment/SubmitCardPayment` | Submits a card payment, with a fee. |
| `POST /SchedulePayment/AchSchedulePayment` | Creates/updates a recurring debit. |
| `POST /payment/scheduledpayments/delete` | Stops a recurring debit. |
| `POST /Payment/CancelPayment` | Cancels a pending payment. |

Run `wabhoa writes` for the full list of 24, including payment-method,
property, and account mutations.

### Read-only helpers used by the payment flow

These are safe reads that a future write implementation would need. They are
not in the write catalog and `wabhoa api` will call them:

`/Payment/BankList` · `/Payment/GetAllPaymentFrequencies` ·
`/Payment/CalendarDateRestriction` · `/Payment/BankPaymentDates` ·
`/ScheduledPayment/GetFuturePaymentDates` · `/payment/ECheckFeeInfo` ·
`/cards/getEstimatedCardFeesAndTotalAmount` · `/GetManagementCompanyById` ·
`/Properties/Search` · `/Properties/MemberVerify` ·
`/payment/CheckDuplicatePayment` · `/CMC/CMCRevenueShareText` ·
`/GetAvailableProductServices`

`/payment/CheckDuplicatePayment` is a read despite living in the payment flow —
it asks whether a payment *would* be a duplicate. `GetStatementByteArray` is
now implemented by `wabhoa documents download` and is documented above with
the rest of the reads.
