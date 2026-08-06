//! Date handling across the portal's three date dialects.
//!
//! The CLI speaks ISO `YYYY-MM-DD` everywhere (SPEC v1), so everything is
//! normalized on the way in and translated on the way out:
//!
//! - JSON responses carry ASP.NET epoch dates — `/Date(1785567600000-0700)/`.
//! - Rendered HTML and the search form use `MM/DD/YYYY`.
//! - .NET's `DateTime.MinValue` (and a 1900-01-01 placeholder) stand in for
//!   "no value", so both are normalized to absent rather than surfaced as
//!   year-0001 dates.

use pk_cli_core::CliError;

/// A start/end date range for payment-history filtering. Unlike the portal's
/// own form, both bounds are optional: no flags means no date filter at all.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct RangeArgs {
    /// Only payments on or after this date, ISO `YYYY-MM-DD`.
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub start: Option<String>,

    /// Only payments on or before this date, ISO `YYYY-MM-DD`.
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub end: Option<String>,
}

impl RangeArgs {
    /// Resolve to the portal's `MM/DD/YYYY` bounds, validating ordering.
    pub fn resolve(&self) -> Result<(Option<String>, Option<String>), CliError> {
        if let (Some(s), Some(e)) = (&self.start, &self.end) {
            if s > e {
                return Err(CliError::Usage(format!("--start {s} is after --end {e}")));
            }
        }
        let start = self.start.as_deref().map(to_portal).transpose()?;
        let end = self.end.as_deref().map(to_portal).transpose()?;
        Ok((start, end))
    }
}

/// Convert ISO `YYYY-MM-DD` to the portal's `MM/DD/YYYY`.
pub fn to_portal(iso: &str) -> Result<String, CliError> {
    let parts: Vec<&str> = iso.split('-').collect();
    let valid = parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit()));
    if !valid {
        return Err(CliError::Usage(format!(
            "expected an ISO date like 2026-01-31, got {iso:?}"
        )));
    }
    Ok(format!("{}/{}/{}", parts[1], parts[2], parts[0]))
}

/// Convert the portal's `MM/DD/YYYY` to ISO. Returns `None` for anything that
/// isn't a well-formed date, including the blank cells the portal renders.
pub fn from_portal(mdy: &str) -> Option<String> {
    let parts: Vec<&str> = mdy.trim().split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let (m, d, y) = (parts[0], parts[1], parts[2]);
    if y.len() != 4 || !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    let iso = format!("{y}-{m:0>2}-{d:0>2}");
    is_placeholder(&iso)
        .then_some(())
        .map_or(Some(iso), |_| None)
}

/// Parse an ASP.NET `/Date(<millis>[±hhmm])/` value into an ISO date.
///
/// The trailing offset is the *server's* timezone rendering of an instant that
/// the millis already express in UTC, so it is deliberately ignored: applying
/// it would shift assessment dates a day backwards for evening timestamps.
pub fn from_dotnet(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix("/Date(")?.strip_suffix(")/")?;
    // Split off a trailing ±hhmm offset without tripping on the leading sign
    // of a negative epoch (dates before 1970, i.e. the .NET sentinels). The
    // `get` keeps an empty or non-ASCII payload from panicking on the slice.
    let millis = match inner.get(1..)?.find(['+', '-']) {
        Some(i) => &inner[..i + 1],
        None => inner,
    };
    let millis: i64 = millis.parse().ok()?;
    let (y, m, d) = civil_from_days(millis.div_euclid(86_400_000));
    let iso = format!("{y:04}-{m:02}-{d:02}");
    (!is_placeholder(&iso)).then_some(iso)
}

/// Whether a date is one of the portal's stand-ins for "unset".
///
/// `0001-01-01` is .NET's `DateTime.MinValue`; `1900-01-01` is the placeholder
/// the payment-options endpoint returns for an association that publishes no
/// assessment schedule.
fn is_placeholder(iso: &str) -> bool {
    iso.starts_with("0001-01-01") || iso.starts_with("1900-01-01")
}

/// Days since the Unix epoch → (year, month, day).
///
/// Deliberately dependency-free: Howard Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_to_portal_format() {
        assert_eq!(to_portal("2026-01-31").unwrap(), "01/31/2026");
        assert_eq!(to_portal("2026-12-05").unwrap(), "12/05/2026");
    }

    #[test]
    fn bad_dates_are_usage_errors() {
        for bad in ["2026-1-5", "01/31/2026", "not-a-date", "2026-01", ""] {
            assert!(to_portal(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn portal_dates_round_trip() {
        assert_eq!(from_portal("08/01/2026").as_deref(), Some("2026-08-01"));
        // Single-digit components appear in some rendered cells.
        assert_eq!(from_portal("8/1/2026").as_deref(), Some("2026-08-01"));
        assert_eq!(from_portal(""), None);
        assert_eq!(from_portal("n/a"), None);
    }

    #[test]
    fn dotnet_epoch_dates_parse() {
        // The payment date shown in the portal as 08/01/2026.
        assert_eq!(
            from_dotnet("/Date(1785567600000-0700)/").as_deref(),
            Some("2026-08-01")
        );
        assert_eq!(from_dotnet("/Date(0+0000)/").as_deref(), Some("1970-01-01"));
        // Offset is optional.
        assert_eq!(from_dotnet("/Date(0)/").as_deref(), Some("1970-01-01"));
    }

    #[test]
    fn dotnet_sentinels_read_as_absent() {
        // DateTime.MinValue — the portal's "never happened" marker.
        assert_eq!(from_dotnet("/Date(-62135596800000-0800)/"), None);
        // The 1900-01-01 placeholder for an unscheduled assessment.
        assert_eq!(from_dotnet("/Date(-2208960000000-0800)/"), None);
        assert_eq!(from_portal("01/01/1900"), None);
    }

    #[test]
    fn malformed_dotnet_dates_are_none() {
        for bad in ["", "/Date()/", "1785567600000", "/Date(abc)/"] {
            assert_eq!(from_dotnet(bad), None, "{bad} should not parse");
        }
    }

    #[test]
    fn civil_dates_match_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // leap year start
        assert_eq!(civil_from_days(20_640), (2026, 7, 6));
    }

    #[test]
    fn range_defaults_to_unfiltered() {
        let (start, end) = RangeArgs::default().resolve().unwrap();
        assert_eq!((start, end), (None, None));
    }

    #[test]
    fn inverted_range_is_rejected() {
        let r = RangeArgs {
            start: Some("2026-06-01".into()),
            end: Some("2026-01-01".into()),
        };
        assert!(r.resolve().is_err());
    }
}
