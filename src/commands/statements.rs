//! `wabhoa statements` — association statement packets, when published.
//!
//! Many associations publish none, in which case the portal renders "NO
//! STATEMENT DATA AVAILABLE" and `list` returns empty. When statements are
//! present, `download <id>` fetches the PDF bytes and `--all -o DIR` batches
//! the fetch into a directory.
//!
//! The download endpoint is `POST /Statements/GetStatementByteArray` (its
//! `IsSuccessful` / base64 `File` envelope is documented in `docs/api.md`).
//! The reauth-wrapped fetch runs list + download in one closure so a mid-run
//! session lapse recovers the token and replays from the top rather than
//! writing a half-empty batch.

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use pk_cli_core::{output, CliError};
use serde_json::{json, Value};

use super::{emit, items, note_empty, table_view, Ctx, STATEMENTS_PAGE};
use crate::parse;

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// List published statements.
    #[command(visible_alias = "ls")]
    List,
    /// Download a statement — one by id, or every one with `--all`
    /// (statement-download/v1, statement-download-batch/v1).
    #[command(visible_alias = "get")]
    Download(DownloadArgs),
}

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// The statement `id` from `statements list`. Omit and pass `--all` for
    /// every published statement.
    pub id: Option<String>,
    /// Download every published statement. Requires `-o DIR` (or the current
    /// directory) — batches can't stream to stdout.
    #[arg(long, conflicts_with = "id")]
    pub all: bool,
    /// Where to write. With an id: a file path, or `-` for stdout. With
    /// `--all`: a directory (created if needed). Default: the statement's
    /// portal filename in the current directory.
    #[arg(short = 'o', long = "output", value_name = "PATH")]
    pub output: Option<String>,
}

pub fn run(ctx: &Ctx, cmd: &Cmd) -> Result<(), CliError> {
    match cmd {
        Cmd::List => list(ctx),
        Cmd::Download(args) => download(ctx, args),
    }
}

fn list(ctx: &Ctx) -> Result<(), CliError> {
    let statements = ctx.read(|c| Ok(parse::statements(&c.get_text(STATEMENTS_PAGE)?)))?;
    if statements.is_empty() {
        note_empty(ctx, "statements");
    }
    emit(
        ctx,
        "statement-list",
        json!({ "statements": statements }),
        |v| {
            output::table(&table_view(
                &items(v, "statements"),
                &["date", "description", "amount", "id"],
            ));
        },
    );
    Ok(())
}

fn download(ctx: &Ctx, args: &DownloadArgs) -> Result<(), CliError> {
    match (args.all, args.id.as_deref()) {
        (true, _) => download_all(ctx, args),
        (false, Some(id)) => download_one(ctx, id, args),
        (false, None) => Err(CliError::Usage(
            "give a statement id (see `wabhoa statements list`) or --all".into(),
        )),
    }
}

fn download_one(ctx: &Ctx, id: &str, args: &DownloadArgs) -> Result<(), CliError> {
    let (statement, bytes) = fetch(ctx, id)?;
    let file_name = file_name_of(&statement, id);

    // `-o -` makes the file itself the stdout data stream; diagnostics stay on
    // stderr so a pipe carries only the PDF.
    if args.output.as_deref() == Some("-") {
        std::io::stdout()
            .write_all(&bytes)
            .map_err(|e| CliError::Upstream(format!("writing statement to stdout: {e}")))?;
        if !ctx.common.quiet {
            eprintln!("wrote {} bytes ({file_name}) to stdout", bytes.len());
        }
        return Ok(());
    }

    let path = resolve_output(args.output.as_deref(), &file_name);
    write_file(&path, &bytes)?;

    emit(
        ctx,
        "statement-download",
        saved_dto(&statement, id, &path, bytes.len()),
        |v| println!("{}", saved_line(v)),
    );
    Ok(())
}

fn download_all(ctx: &Ctx, args: &DownloadArgs) -> Result<(), CliError> {
    if args.output.as_deref() == Some("-") {
        return Err(CliError::Usage(
            "--all can't stream to stdout; give a directory with -o, or omit it".into(),
        ));
    }

    // One session, one reauth boundary: a mid-run expiry replays from the top
    // (nothing is written yet), avoiding a half-written directory.
    let downloaded = ctx.read(|c| {
        let statements = parse::statements(&c.get_text(STATEMENTS_PAGE)?);
        let mut out = Vec::with_capacity(statements.len());
        for s in statements {
            let Some(id) = s
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty())
            else {
                // A statement the portal renders without a working download
                // link is worth listing but can't be fetched — silently skip
                // in --all rather than fail the whole batch.
                continue;
            };
            let bytes = c.download_statement(&id)?;
            out.push((s, id, bytes));
        }
        Ok(out)
    })?;

    let dir = args
        .output
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_default();
    if !dir.as_os_str().is_empty() && !dir.exists() {
        std::fs::create_dir_all(&dir)
            .map_err(|e| CliError::Upstream(format!("creating {}: {e}", dir.display())))?;
    }

    // `file_name_of` derives the on-disk name from date + description, which is
    // friendlier than the opaque server key but is not guaranteed unique — two
    // statements from the same day with descriptions that sanitize to the same
    // string would silently overwrite. Track what has already been written so
    // a collision falls through to a `_2`, `_3`, … suffix (and, if even that
    // collides, to the sanitized `id`, which the portal *does* guarantee is
    // unique per statement).
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut written = Vec::with_capacity(downloaded.len());
    let mut bytes_total: u64 = 0;
    for (statement, id, bytes) in &downloaded {
        let name = disambiguate(&file_name_of(statement, id), id, &used);
        used.insert(name.clone());
        let path = if dir.as_os_str().is_empty() {
            PathBuf::from(&name)
        } else {
            dir.join(&name)
        };
        write_file(&path, bytes)?;
        bytes_total += bytes.len() as u64;
        written.push(saved_dto(statement, id, &path, bytes.len()));
    }

    if written.is_empty() {
        note_empty(ctx, "downloadable statements");
    }
    let where_to = if dir.as_os_str().is_empty() {
        ".".to_string()
    } else {
        dir.display().to_string()
    };
    let payload = json!({
        "count": written.len(),
        "bytes_total": bytes_total,
        "dir": where_to,
        "items": written,
    });
    emit(ctx, "statement-download-batch", payload, |v| {
        for it in v
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            println!("{}", saved_line(it));
        }
        println!(
            "{} statement(s), {} bytes → {}",
            v.get("count").and_then(Value::as_u64).unwrap_or(0),
            v.get("bytes_total").and_then(Value::as_u64).unwrap_or(0),
            v.get("dir").and_then(Value::as_str).unwrap_or("."),
        );
    });
    Ok(())
}

/// One reauth-wrapped closure: list → find the matching row → download bytes.
/// Doing all three inside `ctx.read` means a mid-flight session lapse retries
/// the whole thing rather than downloading against a stale list.
fn fetch(ctx: &Ctx, id: &str) -> Result<(Value, Vec<u8>), CliError> {
    ctx.read(|c| {
        let statements = parse::statements(&c.get_text(STATEMENTS_PAGE)?);
        let statement = statements
            .into_iter()
            .find(|s| s.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| {
                CliError::NotFound(format!(
                    "no statement with id {id} — run `wabhoa statements list`"
                ))
            })?;
        let bytes = c.download_statement(id)?;
        Ok((statement, bytes))
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    std::fs::write(path, bytes)
        .map_err(|e| CliError::Upstream(format!("writing {}: {e}", path.display())))
}

/// The name to save the statement under. The portal's `FileName` — an opaque
/// server key — is unfriendly to filesystems, so a date-and-description name
/// is preferred, falling back to a sanitized form of the raw id.
fn file_name_of(statement: &Value, id: &str) -> String {
    let date = statement
        .get("date")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let description = statement
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    // The id often already ends `.pdf`; strip the extension before sanitizing
    // so the fallback doesn't turn `abc.pdf` into `abc_pdf.pdf`.
    let (id_stem, ext) = split_extension(id);
    let base = match (date, description.is_empty()) {
        ("", true) => sanitize(id_stem),
        ("", false) => sanitize(description),
        (d, true) => d.to_string(),
        (d, false) => format!("{d} {}", sanitize(description)),
    };
    format!("{base}.{}", ext.unwrap_or("pdf"))
}

/// Pick a filename that hasn't been used yet in this batch. Tries the
/// human-friendly `preferred` name first, then `preferred` with `_2`, `_3`, …
/// suffixed, and finally falls back to the opaque `id` (portal-guaranteed
/// unique) if even that ties. Never touches the filesystem — collision
/// detection is scoped to `used`, so a rerun into the same directory happily
/// overwrites its own previous output.
fn disambiguate<S: std::hash::BuildHasher>(
    preferred: &str,
    id: &str,
    used: &std::collections::HashSet<String, S>,
) -> String {
    if !used.contains(preferred) {
        return preferred.to_string();
    }
    let (stem, ext) = split_extension(preferred);
    let ext = ext.unwrap_or("pdf");
    // Cap the search well below any sane batch size so a pathological input
    // still terminates. In practice the second attempt (`_2`) is enough.
    for n in 2..=1024 {
        let candidate = format!("{stem}_{n}.{ext}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    // Final fallback: the opaque id, which the portal guarantees is unique
    // per statement. Its extension is trusted in the same way `file_name_of`
    // trusts it.
    let (id_stem, id_ext) = split_extension(id);
    let unique = format!("{}.{}", sanitize(id_stem), id_ext.unwrap_or("pdf"));
    if !used.contains(&unique) {
        return unique;
    }
    // Nothing left to try; return the id verbatim rather than lose the file.
    id.to_string()
}

/// Split `"abc.pdf"` into `("abc", Some("pdf"))`. Requires the suffix to look
/// like a real extension: 1–5 lowercase-alphanumeric characters, with at least
/// one letter — so `"statement.v2.1"` keeps its whole name and defaults to
/// `.pdf`, rather than being saved as `something.1`.
fn split_extension(id: &str) -> (&str, Option<&str>) {
    let Some(dot) = id.rfind('.') else {
        return (id, None);
    };
    let ext = &id[dot + 1..];
    let is_ext = (1..=5).contains(&ext.len())
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && ext.chars().any(|c| c.is_ascii_alphabetic());
    if is_ext {
        (&id[..dot], Some(ext))
    } else {
        (id, None)
    }
}

/// Make a string safe for a filesystem path segment. Portal-provided
/// descriptions contain spaces and punctuation that most shells tolerate but
/// path separators do not; replace the dangerous ones and collapse runs.
fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_was_gap = false;
    for c in raw.chars() {
        let keep = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '(' | ')');
        if keep {
            out.push(c);
            prev_was_gap = false;
        } else if !prev_was_gap && !out.is_empty() {
            out.push('_');
            prev_was_gap = true;
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "statement".into()
    } else {
        trimmed.to_string()
    }
}

fn saved_dto(statement: &Value, id: &str, path: &Path, bytes: usize) -> Value {
    json!({
        "id": id,
        "date": statement.get("date").cloned().unwrap_or(Value::Null),
        "description": statement.get("description").cloned().unwrap_or(Value::Null),
        "amount": statement.get("amount").cloned().unwrap_or(Value::Null),
        "file_name": statement.get("file_name").cloned().unwrap_or(Value::String(id.to_string())),
        "path": path.display().to_string(),
        "bytes": bytes,
    })
}

fn saved_line(v: &Value) -> String {
    format!(
        "Saved {} → {} ({} bytes)",
        v.get("description")
            .and_then(Value::as_str)
            .unwrap_or("statement"),
        v.get("path").and_then(Value::as_str).unwrap_or("?"),
        v.get("bytes").and_then(Value::as_u64).unwrap_or(0),
    )
}

/// A file path as given, a filename joined onto an existing directory, or the
/// portal's filename in the current directory when nothing was asked for.
fn resolve_output(output: Option<&str>, file_name: &str) -> PathBuf {
    match output {
        None => PathBuf::from(file_name),
        Some(o) => {
            let p = PathBuf::from(o);
            if p.is_dir() {
                p.join(file_name)
            } else {
                p
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn output_defaults_to_a_derived_file_name() {
        assert_eq!(
            resolve_output(None, "2026-01-15 January_2026_Statement.pdf"),
            PathBuf::from("2026-01-15 January_2026_Statement.pdf")
        );
    }

    #[test]
    fn an_explicit_path_is_used_verbatim() {
        assert_eq!(
            resolve_output(Some("/tmp/s.pdf"), "ignored.pdf"),
            PathBuf::from("/tmp/s.pdf")
        );
    }

    #[test]
    fn a_directory_target_gets_the_filename_appended() {
        // The current directory is a stable "is a dir" case in tests.
        assert_eq!(resolve_output(Some("."), "s.pdf"), PathBuf::from("./s.pdf"));
    }

    #[test]
    fn file_name_prefers_date_and_description_over_opaque_id() {
        let s = json!({
            "date": "2026-01-15",
            "description": "January 2026 Statement",
        });
        assert_eq!(
            file_name_of(&s, "9001_SA_222222_2026-01.pdf"),
            "2026-01-15 January_2026_Statement.pdf"
        );
    }

    #[test]
    fn file_name_survives_a_missing_description_or_date() {
        let s = json!({});
        // Falls back to a sanitized form of the id when no metadata is present.
        assert_eq!(file_name_of(&s, "abc.pdf"), "abc.pdf");
        // With a date but no description, use the date.
        let s = json!({ "date": "2026-01-15" });
        assert_eq!(file_name_of(&s, "9001.pdf"), "2026-01-15.pdf");
    }

    #[test]
    fn sanitize_replaces_dangerous_characters() {
        assert_eq!(sanitize("Foo / Bar"), "Foo_Bar");
        assert_eq!(sanitize("  spaced  out  "), "spaced_out");
        assert_eq!(sanitize("/////"), "statement");
        // Parens and dashes survive — they're legal path characters and useful.
        assert_eq!(sanitize("2026 Q1 (draft)"), "2026_Q1_(draft)");
    }

    #[test]
    fn disambiguate_returns_the_preferred_name_when_unused() {
        let used = std::collections::HashSet::<String>::new();
        assert_eq!(
            disambiguate("2026-01-15 January.pdf", "abc.pdf", &used),
            "2026-01-15 January.pdf"
        );
    }

    #[test]
    fn disambiguate_appends_a_counter_before_overwriting() {
        // The concrete bug: two statements from the same day whose
        // descriptions sanitize to the same string would silently overwrite,
        // and the batch summary would still report both as written. The
        // second (and any further) collision gets `_2`, `_3`, … so both PDFs
        // actually reach disk.
        let mut used = std::collections::HashSet::new();
        let a = disambiguate("2026-01-15 January.pdf", "a.pdf", &used);
        used.insert(a.clone());
        let b = disambiguate("2026-01-15 January.pdf", "b.pdf", &used);
        used.insert(b.clone());
        let c = disambiguate("2026-01-15 January.pdf", "c.pdf", &used);

        assert_eq!(a, "2026-01-15 January.pdf");
        assert_eq!(b, "2026-01-15 January_2.pdf");
        assert_eq!(c, "2026-01-15 January_3.pdf");
        // Each output is a distinct path, so nothing overwrites anything.
        let all: std::collections::HashSet<_> = [a, b, c].into_iter().collect();
        assert_eq!(all.len(), 3);
    }

    /// Wire the disambiguator through actual file writes to prove the batch
    /// path preserves both PDFs when two statements share a derived name.
    /// This mirrors the inner loop of `download_all`; the CLI path itself is
    /// covered by the surface tests, but the write loop was where the bug hid.
    #[test]
    fn colliding_derived_names_both_reach_disk() {
        let statements = [
            (
                json!({ "date": "2026-01-15", "description": "January 2026 Statement" }),
                "9001_SA_222222_2026-01_a.pdf",
                b"AAAA".to_vec(),
            ),
            (
                // Same date, description that sanitizes identically — the
                // exact collision the reviewer flagged.
                json!({ "date": "2026-01-15", "description": "January  2026  Statement" }),
                "9001_SA_222222_2026-01_b.pdf",
                b"BBBBBB".to_vec(),
            ),
        ];

        let dir =
            std::env::temp_dir().join(format!("wabhoa-collision-test-{}", std::process::id()));
        // The temp dir may linger from a previous crashed run; start clean so
        // "already exists" doesn't mask a real collision.
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");

        let mut used = std::collections::HashSet::new();
        let mut paths = Vec::new();
        for (statement, id, bytes) in &statements {
            let name = disambiguate(&file_name_of(statement, id), id, &used);
            used.insert(name.clone());
            let path = dir.join(&name);
            std::fs::write(&path, bytes).expect("write pdf");
            paths.push((path, bytes.len()));
        }

        assert_ne!(
            paths[0].0, paths[1].0,
            "the two writes must land on distinct paths"
        );
        for (path, len) in &paths {
            let on_disk = std::fs::read(path).expect("read back");
            assert_eq!(on_disk.len(), *len, "wrong bytes at {}", path.display());
        }
        // Best-effort cleanup so a rerun starts from a clean slate.
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disambiguate_falls_back_to_the_unique_id_when_all_counters_are_taken() {
        // Pathological but well-defined: pre-fill the numbered slots so the
        // fallback fires. The opaque id is portal-guaranteed unique.
        let mut used = std::collections::HashSet::new();
        used.insert("preferred.pdf".to_string());
        for n in 2..=1024 {
            used.insert(format!("preferred_{n}.pdf"));
        }
        let out = disambiguate("preferred.pdf", "9001_SA_222222.pdf", &used);
        assert_eq!(out, "9001_SA_222222.pdf");
    }

    #[test]
    fn split_extension_only_treats_a_letter_bearing_suffix_as_an_extension() {
        assert_eq!(split_extension("statement.pdf"), ("statement", Some("pdf")));
        assert_eq!(split_extension("statement"), ("statement", None));
        // A version-style "extension" isn't one — save the whole name and let
        // the default extension apply.
        assert_eq!(split_extension("statement.v2.1"), ("statement.v2.1", None));
        // A too-long suffix isn't an extension either.
        assert_eq!(
            split_extension("s.notanextension"),
            ("s.notanextension", None)
        );
    }
}
