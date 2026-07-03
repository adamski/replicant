//! `replicant-seed` — push a directory of JSON documents to a Replicant
//! server. Idempotent by a configurable dedupe field (default: `title`).
//!
//! Typical use: bootstrapping a Replicant deployment with a corpus of seed
//! documents. Each *.json file is pushed verbatim through the sync channel,
//! so every seeded document is owned by (and private to) the authenticated
//! seeding account. To publish a public corpus, use the server-side factory
//! backfill (`mix replicant.backfill_factory`), which creates owned public
//! documents.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use replicant_client::Client;
use serde_json::Value;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    name = "replicant-seed",
    about = "Push a directory of JSON documents to a Replicant server",
    long_about = None,
)]
struct Args {
    /// WebSocket server URL (e.g. wss://db.example.com).
    #[arg(long, env = "REPLICANT_SERVER_URL")]
    server: String,

    /// Email used to authenticate the connection.
    #[arg(long, env = "REPLICANT_USER")]
    user: String,

    /// Application API key (rpa_ prefix).
    #[arg(long, env = "REPLICANT_API_KEY")]
    api_key: String,

    /// Application API secret (rps_ prefix).
    #[arg(long, env = "REPLICANT_API_SECRET")]
    api_secret: String,

    /// Directory containing the JSON documents to push (recursively).
    #[arg(long)]
    json_dir: PathBuf,

    /// Local SQLite work directory. A `replicant-seed.sqlite3` file is
    /// created here for the duration of the run.
    #[arg(long, default_value = "./replicant-seed-workdir")]
    db_dir: PathBuf,

    /// Dot-path inside each document used to detect duplicates. The seeder
    /// reads existing docs from the server, indexes them by this field, and
    /// skips local files whose value already exists. Pass an empty string
    /// to disable dedupe (always push).
    #[arg(long, default_value = "title")]
    dedupe_by: String,

    /// Parse and report what would happen, but do not connect or push.
    #[arg(long)]
    dry_run: bool,

    /// Seconds to wait for the initial sync to land before reading the
    /// existing document set.
    #[arg(long, default_value_t = 5)]
    initial_sync_secs: u64,

    /// Seconds to wait after pushing, so writes flush to the server.
    #[arg(long, default_value_t = 5)]
    flush_secs: u64,
}

#[derive(Debug)]
struct LoadedDoc {
    path: PathBuf,
    content: Value,
    dedupe_key: Option<String>,
}

/// Walk `dir` and parse every `*.json` file into a `LoadedDoc`. Bails on the
/// first parse error so we don't half-seed a server.
fn load_docs(dir: &Path, dedupe_by: &str) -> Result<Vec<LoadedDoc>> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let mut docs = Vec::new();
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading {}", entry.path().display()))?;
        let content: Value = serde_json::from_str(&raw)
            .with_context(|| format!("parsing JSON in {}", entry.path().display()))?;
        let dedupe_key = if dedupe_by.is_empty() {
            None
        } else {
            extract_dot_path(&content, dedupe_by).map(key_to_string)
        };
        docs.push(LoadedDoc {
            path: entry.path().to_path_buf(),
            content,
            dedupe_key,
        });
    }

    docs.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(docs)
}

/// Resolve a dot-path like `title` or `metadata.name` against a JSON value.
/// Returns the value at the path, if it exists; `key_to_string` renders it
/// as a dedupe key.
fn extract_dot_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Render a JSON value as a flat string for use as a dedupe key.
/// Strings are returned unwrapped (no surrounding quotes); other primitives
/// are stringified. Objects/arrays serialize to canonical JSON.
fn key_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    let docs = load_docs(&args.json_dir, &args.dedupe_by)?;
    if docs.is_empty() {
        bail!("no JSON documents found under {}", args.json_dir.display());
    }
    println!(
        "Loaded {} document(s) from {}",
        docs.len(),
        args.json_dir.display()
    );
    if !args.dedupe_by.is_empty() {
        let missing: Vec<_> = docs
            .iter()
            .filter(|d| d.dedupe_key.is_none())
            .map(|d| d.path.display().to_string())
            .collect();
        if !missing.is_empty() {
            println!(
                "  Note: {} doc(s) have no value at `{}` and will always be pushed:",
                missing.len(),
                args.dedupe_by
            );
            for p in &missing {
                println!("    - {}", p);
            }
        }
    }

    if args.dry_run {
        for d in &docs {
            let key = d.dedupe_key.as_deref().unwrap_or("<none>");
            println!("  [dry] {} (key={})", d.path.display(), key);
        }
        println!("Dry run — no connection attempted.");
        return Ok(());
    }

    std::fs::create_dir_all(&args.db_dir)
        .with_context(|| format!("creating work dir {}", args.db_dir.display()))?;
    let db_url = format!(
        "sqlite:{}/replicant-seed.sqlite3?mode=rwc",
        args.db_dir.display()
    );

    println!(
        "Connecting to {} as {} (work dir: {})...",
        args.server,
        args.user,
        args.db_dir.display()
    );
    let client = Client::new(
        &db_url,
        &args.server,
        &args.user,
        &args.api_key,
        &args.api_secret,
    )
    .await
    .map_err(|e| anyhow!("Replicant connect failed: {}", e))?;

    println!("Waiting {}s for initial sync...", args.initial_sync_secs);
    tokio::time::sleep(Duration::from_secs(args.initial_sync_secs)).await;

    let existing_keys: HashSet<String> = if args.dedupe_by.is_empty() {
        HashSet::new()
    } else {
        let docs = client
            .get_all_documents()
            .await
            .map_err(|e| anyhow!("get_all_documents failed: {}", e))?;
        docs.iter()
            .filter_map(|d| extract_dot_path(&d.content, &args.dedupe_by).map(key_to_string))
            .collect()
    };
    println!(
        "Server has {} existing doc(s) indexed by `{}`",
        existing_keys.len(),
        if args.dedupe_by.is_empty() {
            "<none>"
        } else {
            args.dedupe_by.as_str()
        }
    );

    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for doc in &docs {
        let label = doc
            .dedupe_key
            .clone()
            .unwrap_or_else(|| doc.path.display().to_string());

        if let Some(ref key) = doc.dedupe_key {
            if existing_keys.contains(key) {
                skipped += 1;
                println!("  [skip] {}", label);
                continue;
            }
        }

        match client.create_document(doc.content.clone()).await {
            Ok(saved) => {
                created += 1;
                println!("  [push] {} (id={})", label, saved.id);
            }
            Err(e) => {
                failed += 1;
                eprintln!("  [fail] {}: {}", label, e);
            }
        }
    }

    println!("Flushing for {}s...", args.flush_secs);
    tokio::time::sleep(Duration::from_secs(args.flush_secs)).await;
    if let Ok(pending) = client.count_pending_sync().await {
        if pending > 0 {
            eprintln!("Warning: {} doc(s) still pending sync at exit.", pending);
        }
    }

    println!(
        "\nDone. created={} skipped={} failed={}",
        created, skipped, failed
    );
    if failed == 0 {
        Ok(())
    } else {
        bail!("{} push(es) failed", failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn local_dedupe_key_matches_server_side_key_for_strings() {
        let dir = std::env::temp_dir().join(format!("replicant-seed-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("doc.json"), r#"{"title":"Partch 43-tone"}"#).unwrap();

        let docs = load_docs(&dir, "title").unwrap();
        std::fs::remove_dir_all(&dir).unwrap();

        // Must equal the key the server-side index derives via key_to_string —
        // unwrapped, no JSON quotes — or dedupe never matches.
        let server_key = key_to_string(&json!("Partch 43-tone"));
        assert_eq!(docs[0].dedupe_key.as_deref(), Some(server_key.as_str()));
        assert_eq!(server_key, "Partch 43-tone");
    }
}
