# Western Alliance Bank HOA payment portal — observed API

Reverse-engineered from the portal's own traffic at
`https://pay.westernalliancebank.com` on **2026-08-06**. Unofficial and
undocumented: Western Alliance Bank publishes no API for this portal, and any
of this can change without notice. `wabhoa api <path> --raw` is the escape
hatch for checking.

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
`/GetAvailableProductServices` · `/Statements/GetStatementByteArray`

`/payment/CheckDuplicatePayment` is a read despite living in the payment flow —
it asks whether a payment *would* be a duplicate. `GetStatementByteArray`
returns statement PDF bytes and is the natural home for a future
`statements download`.
