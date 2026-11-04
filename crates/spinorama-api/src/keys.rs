//! Administration of individually approved API keys.
//!
//! Byte-compatible port of Python `api/keys.py`: same SQLite schema,
//! same `spn_` token format, only the SHA-256 digest is stored.

// Rust guideline compliant 2026-02-21

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};

/// Returns the key database path (`SPINORAMA_API_KEY_DB` or default).
#[must_use]
pub fn database_path() -> PathBuf {
    std::env::var_os("SPINORAMA_API_KEY_DB").map_or_else(
        || PathBuf::from("/home/spin/run/spin-api/accounts/keys.sqlite3"),
        PathBuf::from,
    )
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn digest(token: &str) -> String {
    let hash = Sha256::digest(token.as_bytes());
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_bytes(count: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; count];
    let mut file = std::fs::File::open("/dev/urandom").context("opening /dev/urandom")?;
    std::io::Read::read_exact(&mut file, &mut buf).context("reading /dev/urandom")?;
    Ok(buf)
}

fn token_urlsafe(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Creates the private key database once.
///
/// # Errors
///
/// Fails when the database already exists (matching Python
/// `FileExistsError`) or cannot be created.
pub fn initialize() -> Result<PathBuf> {
    let path = database_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("creating {} (already exists?)", path.display()))?;
    }
    let conn = Connection::open(&path).context("opening new key database")?;
    conn.execute_batch(
        "CREATE TABLE api_keys (
            id TEXT PRIMARY KEY, digest TEXT UNIQUE NOT NULL,
            email TEXT NOT NULL, name TEXT NOT NULL, purpose TEXT NOT NULL,
            created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL,
            revoked_at INTEGER
        )",
    )
    .context("creating api_keys table")?;
    Ok(path)
}

/// Issues a new key; returns `(key_id, token)` (token shown once).
///
/// # Errors
///
/// Returns an error for empty fields, non-positive validity, or
/// database failures.
pub fn issue(email: &str, name: &str, purpose: &str, days: i64) -> Result<(String, String)> {
    if email.trim().is_empty() || name.trim().is_empty() || purpose.trim().is_empty() || days <= 0 {
        bail!("Email, name, purpose and a positive expiry in days are required");
    }
    let token = format!("spn_{}", token_urlsafe(&random_bytes(32)?));
    let key_id_bytes = random_bytes(8)?;
    let key_id: String = key_id_bytes.iter().map(|b| format!("{b:02x}")).collect();
    let now = now_unix();
    let conn = Connection::open(database_path()).context("opening key database")?;
    conn.execute(
        "INSERT INTO api_keys VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        rusqlite::params![
            key_id,
            digest(&token),
            email.trim(),
            name.trim(),
            purpose.trim(),
            now,
            now + days * 86_400
        ],
    )
    .context("inserting API key")?;
    Ok((key_id, token))
}

/// Looks up a bearer token; returns `(key_id, email)` when valid.
///
/// # Errors
///
/// Propagates database errors (missing/corrupt database), which the
/// server maps to 503.
pub fn lookup(token: &str) -> Result<Option<(String, String)>> {
    let conn = Connection::open_with_flags(
        database_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("opening key database read-only")?;
    let mut stmt = conn.prepare(
        "SELECT id, email FROM api_keys
         WHERE digest = ?1 AND revoked_at IS NULL AND expires_at > ?2",
    )?;
    let row = stmt
        .query_row(rusqlite::params![digest(token), now_unix()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .ok();
    Ok(row)
}

/// Revokes a key; returns false when the id is unknown.
///
/// # Errors
///
/// Propagates database errors.
pub fn revoke(key_id: &str) -> Result<bool> {
    let conn = Connection::open(database_path()).context("opening key database")?;
    let changed = conn.execute(
        "UPDATE api_keys SET revoked_at = ?1 WHERE id = ?2",
        rusqlite::params![now_unix(), key_id],
    )?;
    Ok(changed == 1)
}

/// Lists approvals as JSON text (no digests), for the `keys list` CLI.
///
/// # Errors
///
/// Propagates database errors.
pub fn list_json() -> Result<String> {
    let conn = Connection::open_with_flags(
        database_path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .context("opening key database read-only")?;
    let mut stmt = conn.prepare(
        "SELECT id, email, name, purpose, created_at, expires_at, revoked_at
         FROM api_keys ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], |row| {
        let mut obj = serde_json::Map::new();
        for (idx, key) in [
            "id",
            "email",
            "name",
            "purpose",
            "created_at",
            "expires_at",
            "revoked_at",
        ]
        .iter()
        .enumerate()
        {
            let value: rusqlite::types::Value = row.get(idx)?;
            obj.insert(
                (*key).to_owned(),
                match value {
                    rusqlite::types::Value::Null => serde_json::Value::Null,
                    rusqlite::types::Value::Integer(i) => serde_json::Value::from(i),
                    rusqlite::types::Value::Real(f) => serde_json::Value::from(f),
                    rusqlite::types::Value::Text(s) => serde_json::Value::from(s),
                    rusqlite::types::Value::Blob(_) => serde_json::Value::Null,
                },
            );
        }
        Ok(serde_json::Value::Object(obj))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(serde_json::to_string_pretty(&out).unwrap_or_else(|_| "[]".to_owned()))
}

/// Runs the `keys` administration CLI.
///
/// # Errors
///
/// Returns an error message string for the process to print.
pub fn cli(args: &[String]) -> Result<String> {
    match args.first().map(String::as_str) {
        Some("init") => {
            let path = initialize()?;
            Ok(format!("Initialized {}", path.display()))
        }
        Some("issue") => {
            let mut email = "";
            let mut name = "";
            let mut purpose = "";
            let mut days: i64 = 365;
            let mut it = args[1..].iter();
            while let Some(flag) = it.next() {
                let value = it.next().context("--issue needs values")?;
                match flag.as_str() {
                    "--email" => email = value,
                    "--name" => name = value,
                    "--purpose" => purpose = value,
                    "--days" => days = value.parse().context("--days needs a count")?,
                    other => bail!("unknown argument: {other}"),
                }
            }
            let (key_id, token) = issue(email, name, purpose, days)?;
            Ok(format!(
                "Key ID: {key_id}\nAPI key (shown only once): {token}"
            ))
        }
        Some("list") => list_json(),
        Some("revoke") => {
            let id = args.get(1).context("revoke needs a key id")?;
            if revoke(id)? {
                Ok(format!("Revoked {id}"))
            } else {
                bail!("Unknown key ID");
            }
        }
        _ => bail!("usage: spinorama-api keys init|issue|list|revoke"),
    }
}
