//! Offline black-box tests: the command surface, the error contract, and the
//! exit codes. Nothing here touches the network or the keychain.

use assert_cmd::Command;
use predicates::prelude::*;

fn wabhoa() -> Command {
    Command::cargo_bin("wabhoa").expect("binary builds")
}

/// Every top-level command, for the help-tree walk below.
const COMMANDS: &[&str] = &[
    "auth",
    "config",
    "summary",
    "properties",
    "payments",
    "scheduled",
    "methods",
    "notifications",
    "documents",
    "profile",
    "writes",
    "api",
    "self-update",
    "completions",
    "info",
];

#[test]
fn top_level_help_lists_the_surface() {
    let out = wabhoa().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    for cmd in COMMANDS {
        assert!(stdout.contains(cmd), "`{cmd}` missing from --help");
    }
}

/// Rendering a subcommand's help forces clap's debug assertions to run over
/// that subtree, which is what catches conflicting short flags (e.g. an
/// `api -q` colliding with the global `--quiet`).
#[test]
fn every_subcommand_help_renders() {
    for cmd in COMMANDS {
        wabhoa()
            .args([cmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::is_empty().not());
    }
}

#[test]
fn nested_subcommand_help_renders() {
    for (group, sub) in [
        ("auth", "login"),
        ("auth", "status"),
        ("auth", "set-credential"),
        ("config", "set"),
        ("properties", "list"),
        ("properties", "get"),
        ("payments", "list"),
        ("payments", "get"),
        ("scheduled", "list"),
        ("methods", "list"),
        ("notifications", "list"),
        ("documents", "list"),
        ("documents", "download"),
    ] {
        wabhoa().args([group, sub, "--help"]).assert().success();
    }
}

#[test]
fn list_subcommands_all_have_the_ls_alias() {
    for group in [
        "properties",
        "payments",
        "scheduled",
        "methods",
        "notifications",
        "documents",
    ] {
        let out = wabhoa().args([group, "--help"]).assert().success();
        let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
        assert!(stdout.contains("ls"), "`{group}` is missing the `ls` alias");
    }
}

#[test]
fn version_prints_the_crate_version() {
    wabhoa()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn unknown_command_is_a_usage_error() {
    wabhoa().arg("no-such-command").assert().code(2);
}

#[test]
fn info_reports_the_cli_contract() {
    let out = wabhoa().arg("info").assert().success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("info emits JSON");
    assert_eq!(v["name"], "wabhoa");
    assert_eq!(v["auth"]["required"], true);
    assert_eq!(v["auth"]["method"], "password");
    // cli-info/v1 calls the command list "capabilities" — distinct from the
    // `writes` command, which lists portal endpoints this CLI won't call.
    let commands = v["capabilities"].as_array().expect("capabilities array");
    for expected in ["summary", "properties", "payments", "writes"] {
        assert!(
            commands.iter().any(|c| c == expected),
            "`{expected}` missing from info capabilities"
        );
    }
}

/// `writes` is the CLI describing its own scope, so it must work with no
/// session, no keychain entry, and no network.
#[test]
fn writes_catalog_needs_no_session() {
    let out = wabhoa().args(["--json", "writes"]).assert().success();
    let v: serde_json::Value =
        serde_json::from_slice(&out.get_output().stdout).expect("emits JSON");
    assert_eq!(v["schema"], "write-capability-list/v1");
    let caps = v["writes"].as_array().expect("writes array");
    assert!(!caps.is_empty());
    // Nothing in the catalog may claim to be implemented while this CLI is
    // read-only — that flag is the promise the README makes.
    assert!(
        caps.iter().all(|c| c["implemented"] == false),
        "a write claims to be implemented, but this CLI is read-only"
    );
    assert!(
        caps.iter()
            .any(|c| c["path"] == "/Payment/SubmittPayment" && c["category"] == "money"),
        "the payment-submission endpoint must be catalogued"
    );
}

#[test]
fn completions_render_for_every_supported_shell() {
    for shell in ["bash", "zsh", "fish"] {
        wabhoa()
            .args(["completions", shell])
            .assert()
            .success()
            .stdout(predicate::str::contains("wabhoa"));
    }
}

// ---- argument validation happens before any keychain or network access ----

#[test]
fn an_inverted_date_range_is_rejected_without_a_session() {
    wabhoa()
        .args([
            "payments",
            "list",
            "--start",
            "2026-06-01",
            "--end",
            "2026-01-01",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("is after"));
}

#[test]
fn a_non_iso_date_is_rejected_without_a_session() {
    wabhoa()
        .args(["payments", "list", "--start", "06/01/2026"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("ISO date"));
}

#[test]
fn malformed_api_data_is_rejected_without_a_session() {
    wabhoa()
        .args(["api", "/DashboardContent", "--data", "{not json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("valid JSON"));
}

/// The read-only guarantee, enforced at the escape hatch: posting to a known
/// write endpoint must fail before the request is built, regardless of session.
#[test]
fn api_refuses_to_post_to_a_write_endpoint() {
    for path in [
        "/Payment/SubmittPayment",
        "/SchedulePayment/AchSchedulePayment",
        "/Properties/Delete",
        // Casing must not be a bypass.
        "/payment/submittpayment",
    ] {
        wabhoa()
            .args(["api", path, "--data", "{}"])
            .assert()
            .code(6)
            .stderr(predicate::str::contains("read-only"));
    }
}

/// Error messages routinely name a command to run next. A rename that misses
/// one sends the user to a command that doesn't exist — which is exactly what
/// happened when `capabilities` became `writes`, and the write guard kept
/// pointing at the old name.
#[test]
fn error_messages_only_reference_real_commands() {
    let cases: Vec<Vec<&str>> = vec![
        vec!["api", "/Payment/SubmittPayment", "--data", "{}"],
        vec!["payments", "list", "--start", "06/01/2026"],
        vec!["config", "set", "nope", "x"],
        vec!["api", "/DashboardContent", "--data", "{oops"],
    ];
    for args in cases {
        let out = wabhoa().args(&args).assert().failure();
        let stderr = String::from_utf8_lossy(&out.get_output().stderr).to_string();
        // Every `wabhoa <word>` the message suggests must name a real command.
        for mention in stderr.split("wabhoa ").skip(1) {
            let suggested: String = mention
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-')
                .collect();
            if suggested.is_empty() {
                continue;
            }
            assert!(
                COMMANDS.contains(&suggested.as_str()),
                "error for `{args:?}` suggests `wabhoa {suggested}`, which is not a command:\n{stderr}"
            );
        }
    }
}

/// `documents download` validates its arguments before opening a session:
/// `--all` with no output target is fine (writes to cwd), but `--all -o -`
/// can't stream a batch to stdout, and neither can naming no id at all.
#[test]
fn documents_download_argument_errors_need_no_session() {
    wabhoa()
        .args(["documents", "download"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("documents list"));

    wabhoa()
        .args(["documents", "download", "--all", "-o", "-"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stdout"));

    // `--json` and `-o -` would both take stdout; refusing costs no session.
    wabhoa()
        .args(["documents", "download", "9001.pdf", "-o", "-", "--json"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("both write to stdout"));
}

/// The `download` subcommand has to reach the reader named in the error.
#[test]
fn documents_download_has_a_get_alias() {
    let out = wabhoa().args(["documents", "--help"]).assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("get"),
        "expected `download` to expose a `get` alias for parity with sibling CLIs"
    );
}

/// The old `statements` spelling stays as an alias for one major version, so
/// `wabhoa statements …` keeps resolving to `documents`.
#[test]
fn statements_is_kept_as_an_alias_for_documents() {
    // `statements download` with no id hits the same pre-session usage error.
    wabhoa()
        .args(["statements", "download"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("documents list"));
    // And it appears in top-level help as an alias of `documents`.
    let out = wabhoa().arg("--help").assert().success();
    let stdout = String::from_utf8_lossy(&out.get_output().stdout).to_string();
    assert!(
        stdout.contains("statements"),
        "statements alias missing from --help"
    );
}

#[test]
fn an_unknown_config_key_is_a_usage_error() {
    wabhoa()
        .args(["config", "set", "nope", "value"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown config key"));
}

#[test]
fn config_path_resolves_without_a_session() {
    wabhoa()
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("wabhoa"));
}
