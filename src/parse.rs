//! Turn the portal's pages into flat DTOs.
//!
//! Kept separate from `commands/` so every shape has a unit test that runs
//! against a scrubbed markup sample without a network or a session.

use serde_json::{json, Map, Value};

use pk_cli_scrape as scrape;

use crate::dates::{from_dotnet, from_portal};

/// The CSS classes this portal marks table rows and cells with. The generic
/// scanners in `pk-cli-scrape` take these as arguments precisely because they
/// are the provider's naming, not a universal convention.
const ROW_CLASS: &str = "divTableRow";
const CELL_CLASS: &str = "divTableCell";

/// Every table row on a page, in both dialects the portal renders — real
/// `<tr>` elements and `<div class="divTableRow">` stacks, sometimes on the
/// same page.
fn rows(html: &str) -> Vec<String> {
    let mut out = scrape::table_rows(html);
    out.extend(scrape::blocks_with_class(html, "div", ROW_CLASS));
    out
}

/// The cells of a row, in whichever dialect it was rendered.
fn cells(row: &str) -> Vec<String> {
    let tds = scrape::cells(row);
    if tds.is_empty() {
        scrape::cells_with_class(row, "div", CELL_CLASS)
    } else {
        tds
    }
}

/// The signed-in user's site-user ID, a hidden field on the payment-history
/// page. The history search will not filter by user without it.
pub fn site_user_id(history_page: &str) -> Option<String> {
    scrape::input_value(history_page, "idSiteUserLogin").filter(|v| !v.is_empty())
}

/// Properties, from the `idPropertyMember` select on the payment page.
///
/// That select is the only place the portal exposes a property's full identity
/// — management company, association, and account number — which the balance
/// endpoint needs as its key.
pub fn properties(payment_page: &str) -> Vec<Value> {
    let Some(select) = scrape::block_by_id(payment_page, "select", "idPropertyMember") else {
        return Vec::new();
    };
    scrape::elements(&select, "option")
        .into_iter()
        .filter_map(|opt| {
            let id = scrape::attr(&opt.tag, "value").filter(|v| !v.is_empty())?;
            let mut row = Map::new();
            row.insert("id".into(), json!(id));
            row.insert(
                "address".into(),
                json!(scrape::attr(&opt.tag, "attr_Address")
                    .unwrap_or_else(|| opt.text.clone())
                    .trim()
                    .to_string()),
            );
            insert_str(&mut row, "management_company_id", &opt.tag, "attr_MgCoId");
            insert_str(&mut row, "association_id", &opt.tag, "attr_AssocId");
            insert_str(&mut row, "account_number", &opt.tag, "attr_MemberId");
            // `attr_Balance` is deliberately not surfaced here. It is 0 for
            // every association that doesn't publish a balance, which reads as
            // "paid up" rather than "unknown" — and this markup carries no flag
            // to tell the two apart. Balance comes from the payment-options
            // endpoint instead, which does. See `balance()`.
            insert_num(&mut row, "echeck_fee", &opt.tag, "attr_Fee");
            insert_num(&mut row, "debit_fee", &opt.tag, "attr_DebitFee");
            insert_str(&mut row, "owner", &opt.tag, "attr_FullName");
            insert_str(&mut row, "email", &opt.tag, "attr_EmailAddress");
            insert_str(&mut row, "phone", &opt.tag, "attr_Phone");
            if let Some(a) = scrape::attr(&opt.tag, "attr_hasamenities") {
                row.insert("has_amenities".into(), json!(a.eq_ignore_ascii_case("yes")));
            }
            // A stop code means the association has flagged the account; it is
            // blank in the ordinary case, so only surface it when set.
            insert_str(&mut row, "stop_code", &opt.tag, "attr_StopCode");
            Some(Value::Object(row))
        })
        .collect()
}

/// Saved payment methods, from the `idPaymentMethod` select on the payment
/// page. Richer than the Payment Methods page, which omits the method IDs.
pub fn payment_methods(payment_page: &str) -> Vec<Value> {
    let Some(select) = scrape::block_by_id(payment_page, "select", "idPaymentMethod") else {
        return Vec::new();
    };
    scrape::elements(&select, "option")
        .into_iter()
        .filter_map(|opt| {
            let id = scrape::attr(&opt.tag, "value").filter(|v| !v.is_empty())?;
            // The placeholder row carries a "none" type rather than no value.
            let kind = scrape::attr(&opt.tag, "attr_PaymentType")?;
            if kind.eq_ignore_ascii_case("none") {
                return None;
            }
            let (name, mask) = split_mask(&opt.text);
            let mut row = Map::new();
            row.insert("id".into(), json!(id));
            row.insert("name".into(), json!(name));
            if let Some(m) = mask {
                row.insert("mask".into(), json!(m));
            }
            row.insert("type".into(), json!(kind));
            insert_str(&mut row, "account_type", &opt.tag, "attr_PaymentDetails");
            insert_str(&mut row, "name_on_account", &opt.tag, "attr_PaymentNameOn");
            insert_str(&mut row, "expires", &opt.tag, "attr_ExpirationDate");
            insert_str(&mut row, "zip", &opt.tag, "attr_ZipCode");
            Some(Value::Object(row))
        })
        .collect()
}

/// Split `"UMB, NA X-2623"` into `("UMB, NA", Some("2623"))`.
fn split_mask(label: &str) -> (String, Option<String>) {
    match label.rfind(" X-") {
        Some(i) => (
            label[..i].trim().to_string(),
            Some(label[i + 3..].trim().to_string()),
        ),
        None => (label.trim().to_string(), None),
    }
}

/// Scheduled (recurring) payments, from the dashboard partial.
pub fn scheduled_payments(dashboard: &str) -> Vec<Value> {
    rows(dashboard)
        .iter()
        .filter_map(|row| {
            let tag_end = row.find('>').map(|i| i + 1).unwrap_or(row.len());
            let tag = &row[..tag_end];
            let id = scrape::attr(tag, "data-payment-id").filter(|v| !v.is_empty())?;
            let cells = cells(row);
            let mut out = Map::new();
            out.insert("id".into(), json!(id));
            // Cell 1 is the property; cell 0 holds the payment-type icon.
            if let Some(addr) = cells.get(1).filter(|c| !c.is_empty()) {
                out.insert("property".into(), json!(addr.trim()));
            }
            insert_str(&mut out, "type", tag, "paymentType");
            insert_str(&mut out, "frequency", tag, "data-frequency");
            insert_date(&mut out, "next_payment_date", tag, "data-next-date");
            insert_date(&mut out, "end_date", tag, "data-end-date");
            insert_num(&mut out, "amount", tag, "data-amount");
            insert_num(&mut out, "fee", tag, "data-fee");
            Some(Value::Object(out))
        })
        .collect()
}

/// Payment history records, from the search endpoint's JSON.
pub fn payments(search_response: &Value) -> Vec<Value> {
    search_response
        .get("HistoryPaymentsByUserList")
        .and_then(Value::as_array)
        .map(|list| list.iter().map(payment).collect())
        .unwrap_or_default()
}

/// Normalize one payment-history record into the CLI's flat shape.
fn payment(p: &Value) -> Value {
    let mut out = Map::new();
    if let Some(tx) = p.get("TransactionNumber") {
        // The portal types this as a number; the CLI treats it as an opaque
        // identifier, so it is stringified for stable `payments get` matching.
        out.insert("transaction_number".into(), json!(scalar_string(tx)));
    }
    copy_date(&mut out, "payment_date", p, "PaymentDate");
    copy_date(&mut out, "processed_date", p, "ProcessDate");
    copy_str(&mut out, "status", p, "Status");
    copy_str(&mut out, "type", p, "PaymentType");
    copy_str(&mut out, "property", p, "PropAddress");
    copy_str(&mut out, "payment_method", p, "PaymentMoniker");
    copy_num(&mut out, "amount", p, "PaymentAmount");
    copy_num(&mut out, "fee", p, "PaymentFee");
    copy_num(&mut out, "setup_fee", p, "SetupFee");
    copy_num(&mut out, "refund_amount", p, "RefundAmount");
    copy_str(&mut out, "management_company_id", p, "MngmtCoId");
    copy_str(&mut out, "association_id", p, "AssocId");
    copy_str(&mut out, "account_number", p, "MemberId");
    // `PaymentTotal` is 0 on processed records — the portal computes it only
    // while a payment is being composed — so amount + fee is the real total.
    let total =
        num(p.get("PaymentAmount")).unwrap_or(0.0) + num(p.get("PaymentFee")).unwrap_or(0.0);
    out.insert("total".into(), json!(round_cents(total)));
    Value::Object(out)
}

/// Notifications (portal-sent emails), from the notifications page.
///
/// The list is rendered as a table, while the bodies are pushed into a
/// `messagesArray` JavaScript literal — so the two are zipped by row order,
/// which is the order the portal emits both.
pub fn notifications(page: &str) -> Vec<Value> {
    let bodies = message_bodies(page);
    let mut out = Vec::new();
    let mut seen = 0usize;
    for row in rows(page) {
        let cells = cells(&row);
        // Date, subject, to, cc — anything shorter is a header or a spacer.
        if cells.len() < 3 {
            continue;
        }
        let Some(date) = from_portal(&cells[0]) else {
            continue;
        };
        let mut item = Map::new();
        if let Some((id, body)) = bodies.get(seen) {
            item.insert("id".into(), json!(id));
            item.insert("body".into(), json!(scrape::strip_tags(body)));
        }
        seen += 1;
        item.insert("date".into(), json!(date));
        item.insert("subject".into(), json!(cells[1]));
        if let Some(to) = cells.get(2).filter(|c| !c.is_empty()) {
            item.insert("to".into(), json!(to));
        }
        if let Some(cc) = cells.get(3).filter(|c| !c.is_empty()) {
            item.insert("cc".into(), json!(cc));
        }
        out.push(Value::Object(item));
    }
    out
}

/// Pull `(id, html_body)` out of each `messagesArray.push({ Id: N, Message: '…' })`.
fn message_bodies(page: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = page;
    while let Some(i) = rest.find("messagesArray.push({") {
        rest = &rest[i + "messagesArray.push({".len()..];
        let Some(id) = field_after(rest, "Id:") else {
            continue;
        };
        let body = quoted_after(rest, "Message:").unwrap_or_default();
        out.push((id, body));
    }
    out
}

/// The bare (unquoted) value following `label` in a JS object literal.
fn field_after(s: &str, label: &str) -> Option<String> {
    let at = s.find(label)? + label.len();
    let v: String = s[at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (!v.is_empty()).then_some(v)
}

/// The single-quoted string following `label` in a JS object literal.
fn quoted_after(s: &str, label: &str) -> Option<String> {
    let at = s.find(label)? + label.len();
    let rest = s[at..].trim_start();
    let rest = rest.strip_prefix('\'')?;
    // The portal HTML-escapes the body, so an apostrophe inside it arrives as
    // `&#39;` and cannot terminate the literal early.
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

/// Published statement packets, from the statement-history page.
///
/// The test account's associations publish none, so this parses the generic
/// row shape rather than a verified layout; an unrecognized row is skipped
/// instead of guessed at.
pub fn statements(page: &str) -> Vec<Value> {
    rows(page)
        .iter()
        .filter_map(|row| {
            let cells = cells(row);
            if cells.len() < 2 {
                return None;
            }
            let date = from_portal(&cells[0])?;
            let mut out = Map::new();
            out.insert("date".into(), json!(date));
            out.insert("description".into(), json!(cells[1]));
            if let Some(amount) = cells.get(2).and_then(|c| money(c)) {
                out.insert("amount".into(), json!(amount));
            }
            Some(Value::Object(out))
        })
        .collect()
}

/// The account profile. The portal masks the phone and email in the rendered
/// form, so those are returned exactly as masked rather than pretended whole.
pub fn profile(page: &str) -> Value {
    let mut out = Map::new();
    for (key, id) in [
        ("first_name", "txtFirstName"),
        ("last_name", "txtLastName"),
        ("phone", "txtPhoneNumber"),
        ("email", "txtEmailAddress"),
    ] {
        if let Some(v) = scrape::input_value(page, id).filter(|v| !v.is_empty()) {
            out.insert(key.into(), json!(v));
        }
    }
    Value::Object(out)
}

/// The association's payment options and balance, from its JSON endpoint.
pub fn balance(options: &Value) -> Value {
    let mut out = Map::new();
    // The portal hides the balance for associations that don't publish one;
    // reporting 0.00 in that case would read as "paid up", which is a lie.
    let published = options
        .get("ShowDisplayBalance")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    out.insert("balance_published".into(), json!(published));
    if published {
        copy_num(&mut out, "balance", options, "TotalNewBalanceAmountDue");
        copy_num(&mut out, "payment_amount", options, "PaymentAmount");
        copy_str(&mut out, "balance_label", options, "DisplayBalanceText");
    }
    if let Some(d) = options
        .get("NextAssessmentDate")
        .and_then(Value::as_str)
        .and_then(from_dotnet)
    {
        out.insert("next_assessment_date".into(), json!(d));
    }
    copy_str(&mut out, "payment_frequency", options, "PaymentFrequency");
    if let Some(hold) = options.get("IsOnHold").and_then(Value::as_bool) {
        out.insert("on_hold".into(), json!(hold));
    }
    if let Some(name) = options
        .pointer("/SiteContext/ManagementCompany/Name")
        .and_then(Value::as_str)
    {
        out.insert("management_company".into(), json!(name));
    }
    Value::Object(out)
}

// ---- small helpers ---------------------------------------------------------

/// Insert an attribute as a string, skipping it when absent or blank.
fn insert_str(out: &mut Map<String, Value>, key: &str, tag: &str, attr: &str) {
    if let Some(v) = scrape::attr(tag, attr).filter(|v| !v.trim().is_empty()) {
        out.insert(key.into(), json!(v.trim()));
    }
}

/// Insert an attribute parsed as money, skipping it when absent or unparseable.
fn insert_num(out: &mut Map<String, Value>, key: &str, tag: &str, attr: &str) {
    if let Some(v) = scrape::attr(tag, attr).and_then(|v| money(&v)) {
        out.insert(key.into(), json!(v));
    }
}

/// Insert an attribute holding an `MM/DD/YYYY` date, normalized to ISO.
fn insert_date(out: &mut Map<String, Value>, key: &str, tag: &str, attr: &str) {
    if let Some(v) = scrape::attr(tag, attr).as_deref().and_then(from_portal) {
        out.insert(key.into(), json!(v));
    }
}

fn copy_str(out: &mut Map<String, Value>, key: &str, src: &Value, field: &str) {
    if let Some(v) = src.get(field).map(scalar_string).filter(|v| !v.is_empty()) {
        out.insert(key.into(), json!(v));
    }
}

fn copy_num(out: &mut Map<String, Value>, key: &str, src: &Value, field: &str) {
    if let Some(v) = num(src.get(field)) {
        out.insert(key.into(), json!(round_cents(v)));
    }
}

fn copy_date(out: &mut Map<String, Value>, key: &str, src: &Value, field: &str) {
    if let Some(d) = src.get(field).and_then(Value::as_str).and_then(from_dotnet) {
        out.insert(key.into(), json!(d));
    }
}

/// Render a JSON scalar as a plain string (no surrounding quotes).
fn scalar_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => money(s),
        _ => None,
    }
}

/// Parse a money-ish string: `$100.00`, `100.0000`, `1,234.56`, `(5.00)`.
fn money(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let negative = t.starts_with('(') && t.ends_with(')');
    let cleaned: String = t
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();
    let v: f64 = cleaned.parse().ok()?;
    Some(if negative { -v } else { v })
}

/// Round to cents so 100.0000 and float noise both render as 100.0.
fn round_cents(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROPERTY_SELECT: &str = r#"
      <select name="MemberPropertyId" id="idPropertyMember" class="form-control">
        <option value="1111111" attr_Balance="0" attr_Address="1 Sample St " attr_FullName="Sample Owner"
                attr_Phone="(555) 555-0000" attr_EmailAddress="owner@example.com" attr_MgCoId="9001"
                attr_AssocId="SA" attr_AltMemberId="222222" attr_MemberId="222222" attr_Fee="0"
                attr_DebitFee="3.95" attr_StopCode="" attr_hasamenities="no"> 1 Sample St </option>
      </select>
      <select name="MemberPaymentMethodId" id="idPaymentMethod" class="form-control">
        <option value="0" attr_PaymentType="none" attr_PaymentDetails="none">Select a payment method</option>
        <option value="333333" attr_PaymentType="ECheck" attr_PaymentDetails="Checking"
                attr_PaymentNameOn="Sample Owner" attr_ZipCode="" attr_ExpirationDate="">SAMPLE BANK, NA X-0000</option>
      </select>"#;

    #[test]
    fn properties_carry_full_association_identity() {
        let props = properties(PROPERTY_SELECT);
        assert_eq!(props.len(), 1);
        let p = &props[0];
        assert_eq!(p["id"], "1111111");
        assert_eq!(p["address"], "1 Sample St");
        assert_eq!(p["management_company_id"], "9001");
        assert_eq!(p["association_id"], "SA");
        assert_eq!(p["account_number"], "222222");
        // The select's `attr_Balance` is not reported: 0 there means "this
        // association publishes no balance", not "nothing is owed".
        assert!(p.get("balance").is_none());
        assert_eq!(p["debit_fee"], 3.95);
        assert_eq!(p["has_amenities"], false);
        // A blank stop code is omitted rather than surfaced as "".
        assert!(p.get("stop_code").is_none());
    }

    #[test]
    fn payment_methods_drop_the_placeholder_and_split_the_mask() {
        let methods = payment_methods(PROPERTY_SELECT);
        assert_eq!(methods.len(), 1, "the `none` placeholder must be dropped");
        let m = &methods[0];
        assert_eq!(m["id"], "333333");
        assert_eq!(m["name"], "SAMPLE BANK, NA");
        assert_eq!(m["mask"], "0000");
        assert_eq!(m["type"], "ECheck");
        assert_eq!(m["account_type"], "Checking");
    }

    #[test]
    fn a_label_without_a_mask_stays_whole() {
        assert_eq!(
            split_mask("SAMPLE BANK, NA"),
            ("SAMPLE BANK, NA".into(), None)
        );
        assert_eq!(
            split_mask("SAMPLE BANK X-1234"),
            ("SAMPLE BANK".into(), Some("1234".into()))
        );
    }

    #[test]
    fn scheduled_payments_read_the_row_data_attributes() {
        let html = r#"<tbody>
          <tr data-id="sp-2" id="ECheck1" paymentType="ECheck" data-payment-id="1234567"
              data-frequency="Monthly" data-next-date="09/01/2026" data-end-date=""
              data-amount="100.0000" data-fee="0.0000" class="nowrap">
            <td><img src="img/Icon-Check.svg" alt="Check Icon" /></td>
            <td>1 Sample St </td><td>09/01/2026</td><td>Monthly</td><td>$100.00</td>
          </tr></tbody>"#;
        let sched = scheduled_payments(html);
        assert_eq!(sched.len(), 1);
        let s = &sched[0];
        assert_eq!(s["id"], "1234567");
        assert_eq!(s["property"], "1 Sample St");
        assert_eq!(s["frequency"], "Monthly");
        assert_eq!(s["next_payment_date"], "2026-09-01");
        assert_eq!(s["amount"], 100.0);
        // A blank end date means "runs indefinitely", not an epoch date.
        assert!(s.get("end_date").is_none());
    }

    #[test]
    fn rows_without_a_payment_id_are_not_scheduled_payments() {
        assert!(scheduled_payments("<tr><td>Recent payment</td></tr>").is_empty());
    }

    #[test]
    fn payments_normalize_dates_amounts_and_ids() {
        let raw = json!({"HistoryPaymentsByUserList": [{
            "PaymentDate": "/Date(1785567600000-0700)/",
            "ProcessDate": "/Date(1785673080003-0700)/",
            "AuthorizationDate": "/Date(-62135596800000-0800)/",
            "TransactionNumber": 12345678,
            "PaymentAmount": 100.0, "PaymentFee": 1.5, "SetupFee": 0.0,
            "PaymentTotal": 0.0, "RefundAmount": 0.0,
            "PaymentType": "ECheck", "Status": "Processed",
            "PropAddress": "1 Sample St ", "PaymentMoniker": "SAMPLE BANK X-0000",
            "MngmtCoId": "9001", "AssocId": "SA", "MemberId": "222222"
        }]});
        let list = payments(&raw);
        assert_eq!(list.len(), 1);
        let p = &list[0];
        // Transaction numbers are opaque identifiers, so they stringify.
        assert_eq!(p["transaction_number"], "12345678");
        assert_eq!(p["payment_date"], "2026-08-01");
        assert_eq!(p["status"], "Processed");
        assert_eq!(p["amount"], 100.0);
        // PaymentTotal is 0 on processed records, so total is derived.
        assert_eq!(p["total"], 101.5);
    }

    #[test]
    fn payments_tolerate_an_empty_or_missing_list() {
        assert!(payments(&json!({})).is_empty());
        assert!(payments(&json!({"HistoryPaymentsByUserList": []})).is_empty());
    }

    #[test]
    fn notifications_pair_rows_with_message_bodies() {
        let page = r##"
          <div class="divTableBody">
            <div class="divTableRow"><div class="divTableCell">08/01/2026</div>
              <div class="divTableCell"><a href="#" onclick="javascript:showMessageDetail(99);">Confirmation of Payment</a></div>
              <div class="divTableCell">owner@example.com</div><div class="divTableCell"></div></div>
          </div>
          <script>
            var messagesArray = [];
            messagesArray.push({ Id: 99, Message: '<p>Dear Sample Owner,</p><p>Payment received.</p>' });
          </script>"##;
        let list = notifications(page);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "99");
        assert_eq!(list[0]["date"], "2026-08-01");
        assert_eq!(list[0]["subject"], "Confirmation of Payment");
        assert_eq!(list[0]["to"], "owner@example.com");
        assert_eq!(list[0]["body"], "Dear Sample Owner, Payment received.");
        // An empty Cc cell is omitted rather than surfaced as "".
        assert!(list[0].get("cc").is_none());
    }

    #[test]
    fn statement_placeholder_row_is_not_a_statement() {
        let page = r#"<tbody class="divTableBody">
            <tr class="divTableRow">NO STATEMENT DATA AVAILABLE</tr></tbody>"#;
        assert!(statements(page).is_empty());
    }

    #[test]
    fn profile_keeps_the_portal_masking() {
        let page = r#"<input id="txtFirstName" value="Sample" /><input id="txtLastName" value="Owner" />
                      <input id="txtPhoneNumber" value="******0000" /><input id="txtEmailAddress" value="o****@example.com" />
                      <input id="txtOldPassword" value="" />"#;
        let p = profile(page);
        assert_eq!(p["first_name"], "Sample");
        // Masked values pass through as-is; inventing an unmasked one would lie.
        assert_eq!(p["phone"], "******0000");
        assert!(p.get("password").is_none());
    }

    #[test]
    fn balance_is_withheld_when_the_association_publishes_none() {
        let opts = json!({
            "ShowDisplayBalance": false, "TotalNewBalanceAmountDue": 0.0,
            "NextAssessmentDate": "/Date(-2208960000000-0800)/",
            "PaymentFrequency": "", "IsOnHold": false,
            "SiteContext": {"ManagementCompany": {"Name": "Sample Management LLC"}}
        });
        let b = balance(&opts);
        assert_eq!(b["balance_published"], false);
        // A withheld balance must not be reported as 0.00 — that reads as
        // "paid up" when the truth is "the portal doesn't say".
        assert!(b.get("balance").is_none());
        // The 1900-01-01 placeholder is not a real assessment date.
        assert!(b.get("next_assessment_date").is_none());
        assert_eq!(b["management_company"], "Sample Management LLC");
    }

    #[test]
    fn balance_is_reported_when_published() {
        let opts = json!({
            "ShowDisplayBalance": true, "TotalNewBalanceAmountDue": 412.5,
            "PaymentAmount": 100.0, "DisplayBalanceText": "Current Balance",
            "NextAssessmentDate": "/Date(1788246000000-0700)/"
        });
        let b = balance(&opts);
        assert_eq!(b["balance"], 412.5);
        assert_eq!(b["balance_label"], "Current Balance");
        assert_eq!(b["next_assessment_date"], "2026-09-01");
    }

    #[test]
    fn site_user_id_comes_off_the_history_page() {
        let page =
            r#"<input type="hidden" id="idSiteUserLogin" name="SiteUserLogin" value="7654321" />"#;
        assert_eq!(site_user_id(page).as_deref(), Some("7654321"));
        assert_eq!(site_user_id("<html></html>"), None);
        // A present-but-blank field is not a usable ID.
        assert_eq!(
            site_user_id(r#"<input id="idSiteUserLogin" value="" />"#),
            None
        );
    }

    #[test]
    fn money_parses_the_portal_formats() {
        assert_eq!(money("$100.00"), Some(100.0));
        assert_eq!(money("100.0000"), Some(100.0));
        assert_eq!(money("1,234.56"), Some(1234.56));
        assert_eq!(money("(5.00)"), Some(-5.0));
        assert_eq!(money(""), None);
        assert_eq!(money("n/a"), None);
    }
}
