//! Catalog of the portal's mutating endpoints — everything this CLI
//! deliberately does *not* do.
//!
//! Two jobs, one source of truth:
//!
//! 1. `wabhoa writes` prints it, so the gap between "what the portal can
//!    do" and "what this CLI does" is inspectable rather than tribal knowledge.
//! 2. `wabhoa api` refuses to POST to any path listed here, so the raw escape
//!    hatch can't move money by accident.
//!
//! Endpoints were read out of the portal's own JavaScript (`starscream.*.js`,
//! `epay.api.*.js`); see `docs/api.md`. None of them have been called — the
//! paths and payload shapes are transcribed, not tested, and any future
//! implementation must verify them before trusting them.

/// A portal capability this CLI does not implement.
pub struct Capability {
    /// HTTP method the portal's front end uses.
    pub method: &'static str,
    /// Path under the portal host.
    pub path: &'static str,
    /// Grouping for the `writes` table.
    pub category: Category,
    /// What calling it would do.
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// Moves money, now or on a schedule. The highest-stakes group.
    Money,
    /// Adds, edits, or removes a stored bank account or card.
    PaymentMethod,
    /// Adds, edits, or removes a property on the account.
    Property,
    /// Changes the login itself — profile, password, registration.
    Account,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Category::Money => "money",
            Category::PaymentMethod => "payment-method",
            Category::Property => "property",
            Category::Account => "account",
        }
    }
}

/// Every mutating endpoint observed in the portal's front end.
pub const CAPABILITIES: &[Capability] = &[
    // ---- money ----
    Capability {
        method: "POST",
        path: "/Payment/SubmittPayment",
        category: Category::Money,
        description: "Submit a one-time eCheck (ACH) assessment payment. (Portal's own spelling.)",
    },
    Capability {
        method: "POST",
        path: "/Payment/SubmitCardPayment",
        category: Category::Money,
        description: "Submit a debit/credit card payment. Carries a processing fee.",
    },
    Capability {
        method: "POST",
        path: "/Payment/SubmitAmenityPayment",
        category: Category::Money,
        description: "Pay an amenity charge, for associations that bill them.",
    },
    Capability {
        method: "POST",
        path: "/OneTimePaymentAch",
        category: Category::Money,
        description: "One-time ACH payment made without signing in.",
    },
    Capability {
        method: "POST",
        path: "/SchedulePayment/AchSchedulePayment",
        category: Category::Money,
        description: "Create or update a recurring ACH payment — commits to future debits.",
    },
    Capability {
        method: "POST",
        path: "/Payment/CancelPayment",
        category: Category::Money,
        description: "Cancel a submitted payment that has not yet processed.",
    },
    Capability {
        method: "POST",
        path: "/Payment/CancelingPayment",
        category: Category::Money,
        description: "Cancel-payment variant used by a second portal flow.",
    },
    Capability {
        method: "POST",
        path: "/payment/scheduledpayments/delete",
        category: Category::Money,
        description: "Delete a scheduled recurring payment, stopping future debits.",
    },
    Capability {
        method: "POST",
        path: "/scheduledpayments/delete",
        category: Category::Money,
        description: "Unprefixed alias of the scheduled-payment delete route.",
    },
    // ---- payment methods ----
    Capability {
        method: "POST",
        path: "/payment/paymentmethods/addbankaccountpaymentmethod",
        category: Category::PaymentMethod,
        description: "Save a bank account (routing + account number) for future payments.",
    },
    Capability {
        method: "POST",
        path: "/payment/paymentmethods/addcardpaymentmethod",
        category: Category::PaymentMethod,
        description: "Save a debit/credit card for future payments.",
    },
    Capability {
        method: "POST",
        path: "/payment/paymentmethods/update",
        category: Category::PaymentMethod,
        description: "Update a saved payment method.",
    },
    Capability {
        method: "POST",
        path: "/payment/paymentmethods/delete",
        category: Category::PaymentMethod,
        description: "Delete a saved bank account. Breaks any schedule using it.",
    },
    Capability {
        method: "POST",
        path: "/cards/paymentmethods/delete",
        category: Category::PaymentMethod,
        description: "Delete a saved card. Breaks any schedule using it.",
    },
    Capability {
        method: "POST",
        path: "/Propay/CreatePayerId",
        category: Category::PaymentMethod,
        description: "Register a payer ID with the card processor (ProPay).",
    },
    // ---- properties ----
    Capability {
        method: "POST",
        path: "/MemberProperty/Upsert",
        category: Category::Property,
        description: "Add a property to the account, or update an existing one.",
    },
    Capability {
        method: "POST",
        path: "/Properties/Edit",
        category: Category::Property,
        description: "Edit a property's nickname and contact details.",
    },
    Capability {
        method: "POST",
        path: "/Properties/Delete",
        category: Category::Property,
        description: "Remove a property from the account.",
    },
    // ---- account ----
    Capability {
        method: "POST",
        path: "/Account/UpdateProfile",
        category: Category::Account,
        description: "Change the name, phone, or email on the login.",
    },
    Capability {
        method: "POST",
        path: "/Account/ChangePassword",
        category: Category::Account,
        description: "Change the portal password.",
    },
    Capability {
        method: "POST",
        path: "/Account/PasswordReset/Request",
        category: Category::Account,
        description: "Send a password-reset email.",
    },
    Capability {
        method: "POST",
        path: "/Account/Add",
        category: Category::Account,
        description: "Register a new portal login.",
    },
    Capability {
        method: "POST",
        path: "/SiteUser/Setup",
        category: Category::Account,
        description: "Complete new-account setup.",
    },
    Capability {
        method: "POST",
        path: "/auth/logout",
        category: Category::Account,
        description: "Invalidate the portal session server-side. (`auth logout` clears the local \
                      copy instead, which is why this isn't wired up.)",
    },
];

/// Whether a path is a known mutating endpoint.
///
/// Compared case-insensitively and ignoring a trailing slash: the portal's own
/// JavaScript spells the same route `/Payment/…` in one file and `/payment/…`
/// in another, and IIS routing treats them alike — so a guard that only matched
/// one casing would be trivially bypassed by typing the other.
pub fn is_write(path: &str) -> bool {
    let normalized = normalize(path);
    CAPABILITIES.iter().any(|c| normalize(c.path) == normalized)
}

fn normalize(path: &str) -> String {
    // Drop any query string, then the trailing slash, then lowercase.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    path.trim_end_matches('/').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_write_paths_are_recognized() {
        assert!(is_write("/Payment/SubmittPayment"));
        assert!(is_write("/SchedulePayment/AchSchedulePayment"));
        assert!(is_write("/Properties/Delete"));
    }

    #[test]
    fn the_guard_is_case_and_slash_insensitive() {
        // The portal itself spells these routes inconsistently.
        assert!(is_write("/payment/submittpayment"));
        assert!(is_write("/PAYMENT/SubmittPayment/"));
        assert!(is_write("/Payment/SubmittPayment?foo=1"));
    }

    #[test]
    fn read_paths_are_not_writes() {
        for path in [
            "/Payment/PaymentHistorySearch",
            "/Homeowner/PreSelectedPaymentOptions",
            "/DashboardContent",
            "/Payment/MakePayment",
        ] {
            assert!(!is_write(path), "{path} must not be treated as a write");
        }
    }

    #[test]
    fn a_read_path_that_merely_starts_like_a_write_is_allowed() {
        // Substring matching would wrongly block this; the guard matches whole
        // normalized paths.
        assert!(!is_write("/Payment/SubmittPaymentPreview"));
        assert!(!is_write("/Properties/DeleteConfirmationText"));
    }

    #[test]
    fn every_capability_is_documented_and_well_formed() {
        for c in CAPABILITIES {
            assert!(c.path.starts_with('/'), "{} needs a leading slash", c.path);
            assert!(!c.description.is_empty(), "{} needs a description", c.path);
            assert_eq!(c.method, "POST", "{} — unexpected method", c.path);
        }
    }

    #[test]
    fn capability_paths_are_unique() {
        let mut seen: Vec<String> = CAPABILITIES.iter().map(|c| normalize(c.path)).collect();
        seen.sort();
        let before = seen.len();
        seen.dedup();
        assert_eq!(before, seen.len(), "duplicate capability path");
    }
}
