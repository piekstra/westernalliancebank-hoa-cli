//! Portal-specific date handling.
//!
//! The parsing and formatting mechanisms live in `pk_cli_core::dates` — this
//! module holds only what is true of *this* portal:
//!
//! - which sentinels mean "no value" here. `DateTime.MinValue` is .NET's and
//!   the shared crate knows it; **`1900-01-01` is this portal's own**, returned
//!   by the payment-options endpoint for an association that publishes no
//!   assessment schedule.
//! - the `--start` / `--end` range flags for payment history.
//!
//! The CLI speaks ISO `YYYY-MM-DD` at its boundary (SPEC v1) in both
//! directions; the portal's `MM/DD/YYYY` and `/Date(…)/` never leak into a flag
//! or a DTO.

use pk_cli_core::dates::{
    fmt_iso, fmt_mm_slash_dd_yyyy, is_dotnet_min, parse_dotnet, parse_iso, parse_mm_slash_dd_yyyy,
    Civil,
};
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

/// Convert an ISO `YYYY-MM-DD` flag value to the portal's `MM/DD/YYYY`.
pub fn to_portal(iso: &str) -> Result<String, CliError> {
    parse_iso(iso).map(fmt_mm_slash_dd_yyyy)
}

/// Read a rendered `MM/DD/YYYY` cell as an ISO date, treating this portal's
/// placeholder as absent.
pub fn from_portal(mdy: &str) -> Option<String> {
    parse_mm_slash_dd_yyyy(mdy)
        .filter(|c| !is_placeholder(*c))
        .map(fmt_iso)
}

/// Read a `/Date(…)/` JSON timestamp as an ISO date, treating .NET's sentinel
/// and this portal's placeholder as absent.
pub fn from_dotnet(raw: &str) -> Option<String> {
    parse_dotnet(raw)
        .filter(|c| !is_dotnet_min(*c) && !is_placeholder(*c))
        .map(fmt_iso)
}

/// `1900-01-01` — what the payment-options endpoint returns for an association
/// with no scheduled assessment. Surfacing it as a real date would imply an
/// assessment happened over a century ago.
fn is_placeholder(c: Civil) -> bool {
    c == (1900, 1, 1)
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
        for bad in ["01/31/2026", "not-a-date", "2026-01", "", "2026-13-01"] {
            assert!(to_portal(bad).is_err(), "{bad} should be rejected");
        }
    }

    /// `pk-cli-core`'s shared `parse_iso` accepts single-digit month and day.
    /// The value is unambiguous, and matching the family's parser matters more
    /// than a stricter local rule would.
    #[test]
    fn single_digit_components_are_accepted() {
        assert_eq!(to_portal("2026-1-5").unwrap(), "01/05/2026");
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
    }

    /// Both sentinels must read as absent — the shared .NET one, and this
    /// portal's own placeholder, which `pk-cli-core` deliberately doesn't know
    /// about.
    #[test]
    fn both_sentinels_read_as_absent() {
        // DateTime.MinValue — the portal's "never happened" marker.
        assert_eq!(from_dotnet("/Date(-62135596800000-0800)/"), None);
        // 1900-01-01, in both dialects it appears in.
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
