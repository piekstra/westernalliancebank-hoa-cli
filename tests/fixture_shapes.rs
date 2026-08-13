//! Contract tests: run the real parsers over scrubbed captures of real portal
//! responses.
//!
//! The unit tests in `src/parse.rs` use hand-written markup, which proves the
//! logic but not the assumption underneath it — that the portal still renders
//! what it rendered when the parsers were written. These run against actual
//! captured bytes, so if the portal renames `attr_MgCoId`, drops
//! `data-payment-id`, or stops emitting `/Date(…)/`, this fails loudly instead
//! of quietly producing empty tables.
//!
//! Read `tests/fixtures/README.md` before touching a fixture.

use serde_json::Value;
use wabhoa::parse;

fn text(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"))
}

fn json(name: &str) -> Value {
    let raw = text(name);
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parsing {name}: {e}"))
}

#[test]
fn properties_parse_from_the_real_payment_page() {
    let props = parse::properties(&text("make_payment_selects.html"));
    assert_eq!(props.len(), 1, "the capture holds one property");
    let p = &props[0];
    // The identity triple the balance endpoint is keyed on — losing any one of
    // these silently breaks `properties get` and `summary`.
    assert!(p["management_company_id"].is_string());
    assert!(p["association_id"].is_string());
    assert!(p["account_number"].is_string());
    assert!(p["id"].is_string());
    assert_eq!(p["address"], "1 Sample St");
    // Trailing whitespace in the portal's address attribute must be trimmed.
    assert!(!p["address"].as_str().unwrap().ends_with(' '));
    assert!(p.get("balance").is_none(), "see parse::properties on why");
}

#[test]
fn payment_methods_parse_from_the_real_payment_page() {
    let methods = parse::payment_methods(&text("make_payment_selects.html"));
    assert_eq!(methods.len(), 2, "two saved methods, placeholder dropped");
    for m in &methods {
        assert!(m["id"].is_string());
        assert!(m["name"].is_string());
        assert_eq!(m["type"], "ECheck");
        // The last four is what makes a method identifiable to a human.
        assert!(m["mask"].is_string(), "mask missing from {m}");
        assert!(!m["name"].as_str().unwrap().contains(" X-"));
    }
}

#[test]
fn scheduled_payments_parse_from_the_real_dashboard() {
    let sched = parse::scheduled_payments(&text("dashboard_scheduled.html"));
    assert_eq!(sched.len(), 1);
    let s = &sched[0];
    assert!(s["id"].is_string());
    assert_eq!(s["frequency"], "Monthly");
    assert_eq!(s["type"], "ECheck");
    assert!(s["amount"].is_number());
    // Dates reach the CLI boundary as ISO, never the portal's MM/DD/YYYY.
    let next = s["next_payment_date"].as_str().expect("next payment date");
    assert_eq!(next.len(), 10, "expected ISO YYYY-MM-DD, got {next}");
    assert_eq!(next.matches('-').count(), 2, "expected ISO, got {next}");
    assert!(s["property"].is_string());
}

#[test]
fn payments_parse_from_the_real_search_response() {
    let list = parse::payments(&json("payment_history_search.json"));
    assert_eq!(list.len(), 3);
    for p in &list {
        // Stringified so `payments get` matches on an exact, stable key.
        assert!(
            p["transaction_number"].is_string(),
            "transaction number must be a string"
        );
        assert!(p["amount"].is_number());
        assert!(p["total"].is_number());
        assert!(p["status"].is_string());
        let date = p["payment_date"].as_str().expect("payment date");
        assert_eq!(date.len(), 10, "expected ISO YYYY-MM-DD, got {date}");
    }
    // Newest first, as the portal returns them.
    assert!(list[0]["payment_date"].as_str() >= list[1]["payment_date"].as_str());
}

#[test]
fn the_dotnet_min_date_sentinel_never_reaches_a_dto() {
    // Every record in the capture carries an AuthorizationDate of
    // DateTime.MinValue. Year 0001 must not surface as a date.
    let raw = text("payment_history_search.json");
    assert!(
        raw.contains("-62135596800000"),
        "fixture no longer exercises the sentinel — pick another capture"
    );
    let list = parse::payments(&json("payment_history_search.json"));
    for p in &list {
        for (key, value) in p.as_object().expect("object") {
            if let Some(s) = value.as_str() {
                assert!(!s.starts_with("0001-"), "{key} leaked a sentinel date");
                assert!(!s.starts_with("1900-01-01"), "{key} leaked a placeholder");
            }
        }
    }
}

#[test]
fn balance_parses_from_the_real_options_response() {
    let b = parse::balance(&json("payment_options.json"));
    // This association publishes no balance, so none may be reported.
    assert_eq!(b["balance_published"], false);
    assert!(
        b.get("balance").is_none(),
        "an unpublished balance must be absent, not 0.00"
    );
    assert!(
        b.get("next_assessment_date").is_none(),
        "the 1900-01-01 placeholder is not an assessment date"
    );
    assert!(b["management_company"].is_string());
}

#[test]
fn notifications_parse_from_the_real_page() {
    let list = parse::notifications(&text("notifications.html"));
    assert_eq!(list.len(), 2);
    for n in &list {
        assert!(n["id"].is_string());
        assert!(n["subject"].is_string());
        let date = n["date"].as_str().expect("date");
        assert_eq!(date.len(), 10, "expected ISO YYYY-MM-DD, got {date}");
        // Bodies arrive as HTML and must be flattened to readable text.
        let body = n["body"].as_str().expect("body");
        assert!(!body.is_empty());
        assert!(!body.contains('<'), "body still carries markup: {body}");
        assert!(!body.contains("&#"), "body still carries entities: {body}");
    }
    // IDs must be distinct, or `notifications get` returns the wrong message.
    assert_ne!(list[0]["id"], list[1]["id"]);
}

#[test]
fn profile_parses_from_the_real_page() {
    let p = parse::profile(&text("profile.html"));
    assert_eq!(p["first_name"], "Sample");
    assert_eq!(p["last_name"], "Owner");
    // The portal masks these; the CLI reports the mask rather than inventing
    // an unmasked value it does not have.
    assert!(p["phone"].as_str().unwrap().contains('*'));
    assert!(p["email"].as_str().unwrap().contains('*'));
}

#[test]
fn statements_parse_from_a_populated_history_page() {
    let list = parse::statements(&text("statement_history_published.html"));
    assert_eq!(list.len(), 4, "four rows in the fixture");

    // documents/v1 shape: rows with a `DownloadStatement(...)` handler carry the
    // opaque file name as `id` — that is what `documents download` posts to the
    // byte-array endpoint — plus `name` (the alias), `category`, and `amount`.
    let jan = &list[0];
    assert_eq!(jan["date"], "2026-01-15");
    assert_eq!(jan["name"], "January 2026 Statement");
    assert_eq!(jan["category"], "statement");
    assert_eq!(jan["amount"], 100.0);
    assert_eq!(jan["id"], "9001_SA_222222_2026-01_statement.pdf");

    // The portal HTML-escapes attribute values, so an apostrophe in the alias
    // arrives as `&#39;`. Missing the decode would leave a garbled name and —
    // worse — a garbled file name posted to the endpoint.
    let feb = &list[1];
    assert_eq!(feb["name"], "O'Sample Q1 Statement");
    assert_eq!(feb["id"], "9001_SA_222222_2026-02_statement.pdf");

    // A row without a working download handler is worth listing (the user asked
    // to see everything published), but documents/v1 needs an `id` on every
    // item — so it lists with an empty id (present, but nothing to POST).
    let mar = &list[2];
    assert_eq!(mar["name"], "March 2026 Statement (archived)");
    assert_eq!(mar["id"], "");

    // A *linked* row whose alias is parenthesized. Locating the handler's
    // closing `)` without tracking quotes truncates the alias mid-word, and
    // because that alias becomes both the listed name and the saved PDF's
    // filename, the corruption would be silent in every output.
    let apr = &list[3];
    assert_eq!(apr["name"], "April 2026 Statement (revised)");
    assert_eq!(apr["id"], "9001_SA_222222_2026-04_statement.pdf");
}

#[test]
fn statements_conform_to_the_documents_v1_profile() {
    // Every listed row round-trips into the canonical documents/v1 `Document`
    // (the `amount` provider-extra is ignored on deserialize), proving the wire
    // shape matches the shared profile.
    let list = parse::statements(&text("statement_history_published.html"));
    assert_eq!(list.len(), 4);
    for item in &list {
        let doc: pk_cli_documents::Document = serde_json::from_value(item.clone())
            .unwrap_or_else(|e| panic!("not documents/v1-conformant: {e}\n{item:#}"));
        assert!(!doc.name.is_empty());
        // `id` is present on every item (empty for a non-downloadable row).
        assert!(item.get("id").is_some());
    }
}

#[test]
fn site_user_id_parses_from_the_real_history_page() {
    let id = parse::site_user_id(&text("payment_history_page.html"));
    assert!(id.is_some(), "the history search cannot run without this");
    assert!(id.unwrap().chars().all(|c| c.is_ascii_digit()));
}

/// Every parser must survive markup it doesn't recognize — a portal redesign
/// should empty a table, not crash the CLI.
#[test]
fn parsers_tolerate_unrecognized_markup() {
    for junk in [
        "",
        "<html><body>Maintenance</body></html>",
        "not html at all",
    ] {
        assert!(parse::properties(junk).is_empty());
        assert!(parse::payment_methods(junk).is_empty());
        assert!(parse::scheduled_payments(junk).is_empty());
        assert!(parse::notifications(junk).is_empty());
        assert!(parse::statements(junk).is_empty());
        assert!(parse::site_user_id(junk).is_none());
        assert!(parse::profile(junk).as_object().unwrap().is_empty());
    }
    assert!(parse::payments(&Value::Null).is_empty());
    assert!(parse::balance(&Value::Null).is_object());
}

/// Enforces `tests/fixtures/README.md`. Patterns, not real values — a denylist
/// of actual account numbers would itself be the leak.
#[test]
fn fixtures_carry_no_real_identifiers() {
    // Real values from the account these captures came from, plus the shapes
    // of secrets that must never be committed.
    const BANNED: &[&str] = &[
        "Piekstra",
        "Caleb",
        "piekstracaleb",
        "gmail.com",
        "Enzi",
        "TRADITION",
        "Castle",
        "SOFI",
        // The bare portal hostname is public and appears in message bodies;
        // it's the email form that would carry a real address.
        "@westernalliancebank.com",
        // Cloudflare hex-encodes the real recipient into this attribute, so a
        // capture can leak an address that looks redacted on screen.
        "data-cfemail",
        "ss-id=",
        "ss-pid=",
        "opayUC=",
        "Set-Cookie",
        "password",
    ];

    let dir = format!("{}/tests/fixtures", env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).expect("fixtures dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().is_none_or(|e| e == "md") {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("readable fixture");
        let lower = body.to_lowercase();
        for needle in BANNED {
            assert!(
                !lower.contains(&needle.to_lowercase()),
                "{} contains {needle:?} — see tests/fixtures/README.md",
                path.display()
            );
        }
        // A US phone number or a 9-digit routing number should never appear.
        assert!(
            !body.contains("360326") && !body.contains("(360)"),
            "{} contains a real phone number",
            path.display()
        );

        // Notification bodies embed one-time `ExpressLogin` and `CancelPayment`
        // links. An ExpressLogin token is a *working credential* for the
        // account, so any UUID that isn't an obvious dummy is a live secret.
        // (gitleaks caught exactly this before the first push.)
        for uuid in uuids(&body) {
            assert!(
                uuid.starts_with("00000000-"),
                "{} contains a live token/UUID — replace it with a 00000000-… dummy",
                path.display()
            );
        }
        checked += 1;
    }
    assert!(checked >= 6, "expected the full fixture set, saw {checked}");
}

/// Every `8-4-4-4-12` hex run in `s`. Hand-rolled so the test suite needs no
/// regex dependency.
fn uuids(s: &str) -> Vec<String> {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let bytes: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    'outer: while i < bytes.len() {
        let mut j = i;
        for (g, len) in GROUPS.iter().enumerate() {
            if g > 0 {
                if bytes.get(j) != Some(&'-') {
                    i += 1;
                    continue 'outer;
                }
                j += 1;
            }
            for _ in 0..*len {
                match bytes.get(j) {
                    Some(c) if c.is_ascii_hexdigit() && !c.is_ascii_uppercase() => j += 1,
                    _ => {
                        i += 1;
                        continue 'outer;
                    }
                }
            }
        }
        out.push(bytes[i..j].iter().collect());
        i = j;
    }
    out
}

#[test]
fn uuid_scanner_finds_what_it_should() {
    let found =
        uuids("a=00000000-0000-4000-8000-000000000000 b=deadbeef-1111-4222-8333-444455556666");
    assert_eq!(found.len(), 2);
    assert!(found[1].starts_with("deadbeef"));
    // Too-short groups and non-hex must not match.
    assert!(uuids("1234-56-78-90-12").is_empty());
    assert!(uuids("zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz").is_empty());
}
