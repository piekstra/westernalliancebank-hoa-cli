//! `wabhoa` — piekstra-family CLI for the Western Alliance Bank community
//! association (HOA) assessment payment portal.
//!
//! Conforms to piekstra-cli/1. Read-only today: every command observes, none
//! mutate. The portal's write surface — payments, schedules, saved payment
//! methods, profile edits — is catalogued in `src/writes.rs` and printed by
//! `wabhoa writes`, deliberately unimplemented.

use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use pk_cli_auth::{AuthStatus, LoginArgs, LogoutArgs, SetCredentialArgs};
use pk_cli_config::ConfigStore;
use pk_cli_core::info::{AuthInfo, CliInfo};
use pk_cli_core::{output, CliError, CommonArgs};
use pk_cli_secrets::CredentialStore;
use pk_cli_selfupdate::{SelfUpdateArgs, Updater};

use wabhoa::client::establish_session;
use wabhoa::commands::{
    api, documents, methods, notifications, payments, profile, properties, scheduled, summary,
    writes, Ctx,
};
use wabhoa::config::{self, Config, KEYCHAIN_ACCOUNT, SESSION_ACCOUNT};

const BIN: &str = "wabhoa";
const REPO: &str = "piekstra/westernalliancebank-hoa-cli";

/// Western Alliance Bank HOA assessment portal from the command line —
/// properties, balances, payment history, and schedules. Read-only. Unofficial.
#[derive(Parser, Debug)]
#[command(name = BIN, version, about, long_about = None)]
struct Cli {
    #[command(flatten)]
    common: CommonArgs,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Portal login, session status, and credential management.
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Non-secret settings.
    #[command(subcommand)]
    Config(ConfigCmd),
    /// Everything at a glance: balances, schedules, recent payments.
    Summary,
    /// Properties this login pays assessments for.
    #[command(subcommand)]
    Properties(properties::Cmd),
    /// Assessment payment history.
    #[command(subcommand)]
    Payments(payments::Cmd),
    /// Recurring payments the portal makes automatically.
    #[command(subcommand)]
    Scheduled(scheduled::Cmd),
    /// Saved payment methods, as the portal masks them.
    #[command(subcommand)]
    Methods(methods::Cmd),
    /// Payment notices the portal emailed out.
    #[command(subcommand)]
    Notifications(notifications::Cmd),
    /// Published documents — association statement packets (documents/v1).
    #[command(subcommand, visible_alias = "statements")]
    Documents(documents::Cmd),
    /// The account holder on file.
    Profile,
    /// Portal write endpoints this CLI deliberately does not implement.
    Writes,
    /// Raw portal API passthrough.
    Api(api::ApiArgs),
    /// Update to the latest release from GitHub.
    SelfUpdate(SelfUpdateArgs),
    /// Print a shell completion script.
    Completions { shell: Shell },
    /// Machine-readable capability discovery (cli-info/v1).
    Info,
}

#[derive(Subcommand, Debug)]
enum AuthCmd {
    /// Log in to the portal and cache the session.
    Login(LoginArgs),
    /// Report credential and session state (auth-status/v1).
    Status,
    /// Clear the cached session; --forget also removes the stored password.
    Logout(LogoutArgs),
    /// Raw keychain write for rotation / headless setup.
    SetCredential(SetCredentialArgs),
}

#[derive(Subcommand, Debug)]
enum ConfigCmd {
    /// Print the resolved config file path.
    Path,
    /// Show the effective configuration.
    Show,
    /// Set a config key (base_url, username).
    Set { key: String, value: String },
    /// Remove a config key.
    Unset { key: String },
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(&cli) {
        std::process::exit(output::fail(&e, cli.common.json));
    }
}

fn run(cli: &Cli) -> Result<(), CliError> {
    let store = ConfigStore::new(BIN);
    let creds = CredentialStore::for_binary(BIN);
    let cfg: Config = store.load()?;
    let ctx = Ctx {
        common: &cli.common,
        cfg: &cfg,
        creds: &creds,
    };

    match &cli.command {
        Command::Auth(cmd) => auth(cli, cmd, &store, &creds, &cfg),
        Command::Config(cmd) => config_cmd(cli, cmd, &store),
        Command::Summary => summary::run(&ctx),
        Command::Properties(cmd) => properties::run(&ctx, cmd),
        Command::Payments(cmd) => payments::run(&ctx, cmd),
        Command::Scheduled(cmd) => scheduled::run(&ctx, cmd),
        Command::Methods(cmd) => methods::run(&ctx, cmd),
        Command::Notifications(cmd) => notifications::run(&ctx, cmd),
        Command::Documents(cmd) => documents::run(&ctx, cmd),
        Command::Profile => profile::run(&ctx),
        Command::Writes => writes::run(&ctx),
        Command::Api(args) => api::run(&ctx, args),
        Command::SelfUpdate(args) => Updater {
            repo: REPO.into(),
            binary: BIN.into(),
            target: env!("BUILD_TARGET").into(),
            current: env!("CARGO_PKG_VERSION").into(),
        }
        .run(args, cli.common.json, cli.common.quiet),
        Command::Completions { shell } => {
            clap_complete::generate(*shell, &mut Cli::command(), BIN, &mut std::io::stdout());
            Ok(())
        }
        Command::Info => {
            let info = CliInfo::new(
                BIN,
                env!("CARGO_PKG_VERSION"),
                &format!("https://github.com/{REPO}"),
                AuthInfo {
                    required: true,
                    method: "password".into(),
                    login_hint: Some(format!("{BIN} auth login")),
                },
                &[
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
                ],
            )
            .with_profiles(&[pk_cli_documents::PROFILE]);
            output::json(&serde_json::to_value(&info).unwrap());
            Ok(())
        }
    }
}

fn auth(
    cli: &Cli,
    cmd: &AuthCmd,
    store: &ConfigStore,
    creds: &CredentialStore,
    cfg: &Config,
) -> Result<(), CliError> {
    match cmd {
        AuthCmd::Login(args) => login(cli, args, creds, cfg),
        AuthCmd::Status => {
            let has_password = creds.get(KEYCHAIN_ACCOUNT)?.is_some();
            let has_session = creds.get(SESSION_ACCOUNT)?.is_some();
            let mut status = AuthStatus::new(true, has_session, pk_cli_auth::AuthMethod::Password);
            status.username = cfg.username();
            status.credential_in_keychain = Some(has_password);
            // A stored session is necessary but not sufficient — the portal may
            // have expired it server-side; the first read is the real check.
            status.authenticated = has_session;
            status.emit(cli.common.json);
            Ok(())
        }
        AuthCmd::Logout(args) => {
            creds.delete(SESSION_ACCOUNT)?;
            if args.forget {
                creds.delete(KEYCHAIN_ACCOUNT)?;
                store.clear()?;
                if !cli.common.quiet {
                    eprintln!("session cleared; password removed");
                }
            } else if !cli.common.quiet {
                eprintln!("session cleared (password kept; use --forget to remove it)");
            }
            Ok(())
        }
        AuthCmd::SetCredential(args) => {
            if creds.get(KEYCHAIN_ACCOUNT)?.is_some() && !args.overwrite {
                return Err(CliError::Usage(
                    "a password is already stored; pass --overwrite to replace it".into(),
                ));
            }
            let secret = args.source.read(None)?;
            creds.set(KEYCHAIN_ACCOUNT, &secret)?;
            if !cli.common.quiet {
                eprintln!("password stored in the OS keychain ({})", creds.service());
            }
            Ok(())
        }
    }
}

/// Full portal login. The portal has no second factor: a correct password
/// returns the session cookies directly.
fn login(
    cli: &Cli,
    args: &LoginArgs,
    creds: &CredentialStore,
    cfg: &Config,
) -> Result<(), CliError> {
    let username = cfg.username().ok_or_else(|| {
        CliError::Usage(format!(
            "no portal email configured — run `{BIN} config set username <you@example.com>`"
        ))
    })?;

    // Take the password from the keychain, falling back to the standard
    // ingestion flags so a first run can supply it inline.
    let password = match creds.get(KEYCHAIN_ACCOUNT)? {
        Some(p) if !args.overwrite => p,
        _ => {
            let prompt = if args.non_interactive {
                None
            } else {
                Some("Portal password")
            };
            let secret = args.source.read(prompt)?;
            creds.set(KEYCHAIN_ACCOUNT, &secret)?;
            secret
        }
    };

    establish_session(cfg, creds, &username, &password)?;
    if !cli.common.quiet {
        eprintln!("session cached in the OS keychain ({})", creds.service());
    }
    Ok(())
}

fn config_cmd(cli: &Cli, cmd: &ConfigCmd, store: &ConfigStore) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Path => {
            println!("{}", store.path()?.display());
            Ok(())
        }
        ConfigCmd::Show => {
            let cfg: Config = store.load()?;
            let v = serde_json::to_value(&cfg).unwrap_or_default();
            if cli.common.json {
                output::json(&v);
            } else {
                output::render(&v);
            }
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let mut cfg: Config = store.load()?;
            match key.as_str() {
                "base_url" => cfg.base_url = Some(value.clone()),
                "username" => cfg.username = Some(value.clone()),
                "auto_login" => cfg.auto_login = Some(parse_bool(value)?),
                other => return Err(unknown_key(other)),
            }
            store.save(&cfg)
        }
        ConfigCmd::Unset { key } => {
            let mut cfg: Config = store.load()?;
            match key.as_str() {
                "base_url" => cfg.base_url = None,
                "username" => cfg.username = None,
                "auto_login" => cfg.auto_login = None,
                other => return Err(unknown_key(other)),
            }
            store.save(&cfg)
        }
    }
}

/// Parse a boolean config value, accepting the spellings a user actually
/// types rather than only Rust's.
fn parse_bool(v: &str) -> Result<bool, CliError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        other => Err(CliError::Usage(format!(
            "expected a boolean (true/false), got `{other}`"
        ))),
    }
}

fn unknown_key(key: &str) -> CliError {
    CliError::Usage(format!(
        "unknown config key `{key}` (known: {})",
        config::KNOWN_KEYS.join(", ")
    ))
}
