// SQLCipher-encrypted database setup. The DB is encrypted at rest with a random 32-byte key
// kept in the OS keychain (with a 0600 key-file fallback where no keychain is available); the
// key is passed to SQLite via `PRAGMA key`. We run migrations ourselves (the app no longer uses
// tauri-plugin-sql), and on a DB that can't be decrypted — an upgrade from an older plaintext
// build, or a lost key — we start fresh (the app is configured to reset rather than migrate).

use crate::migration::{get_migrations, MigrationKind};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions};
use std::path::Path;
use std::time::Duration;

const KEYRING_SERVICE: &str = "dev.nutler.app";
const KEYRING_USER: &str = "db-key-v1";

fn random_hex_key() -> String {
    let mut buf = [0u8; 32];
    getrandom::getrandom(&mut buf).expect("OS RNG unavailable");
    let mut s = String::with_capacity(64);
    for b in buf {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The RHS of `PRAGMA key = …` for a raw hex key: SQLCipher wants a quoted blob literal,
/// i.e. the string `"x'<hex>'"` (double quotes included, sqlx inserts the value verbatim).
fn key_pragma_value(hex: &str) -> String {
    format!("\"x'{}'\"", hex)
}

/// The 32-byte at-rest DB key as hex. Prefers the OS keychain; falls back to a 0600 key file
/// beside the DB where no keychain is available (still keeps the key out of the DB file itself).
fn db_key_hex(app_dir: &Path) -> String {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if let Ok(k) = entry.get_password() {
            if is_hex64(&k) {
                return k;
            }
        }
        let k = random_hex_key();
        if entry.set_password(&k).is_ok() {
            tracing::info!("Database key stored in the OS keychain");
            return k;
        }
        tracing::warn!("OS keychain unavailable; falling back to a 0600 key file");
    }

    let key_path = app_dir.join("nutler.key");
    if let Ok(k) = std::fs::read_to_string(&key_path) {
        if is_hex64(k.trim()) {
            return k.trim().to_string();
        }
    }
    let k = random_hex_key();
    let _ = std::fs::write(&key_path, &k);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
    }
    k
}

fn base_opts(db_path: &Path, key_pragma: &str) -> SqliteConnectOptions {
    SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .pragma("key", key_pragma.to_string())
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(5))
}

/// Can the existing DB file be decrypted with this key? (A read forces the `PRAGMA key` +
/// header check; a plaintext or wrong-key file fails here.)
async fn can_decrypt(db_path: &Path, key_pragma: &str) -> bool {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false)
        .pragma("key", key_pragma.to_string());
    match SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
    {
        Ok(pool) => {
            let ok = sqlx::query("SELECT count(*) FROM sqlite_master")
                .fetch_one(&pool)
                .await
                .is_ok();
            pool.close().await;
            ok
        }
        Err(_) => false,
    }
}

fn remove_db_files(db_path: &Path) {
    let _ = std::fs::remove_file(db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
}

/// Apply pending `Up` migrations, tracked in a `_migrations` table so they run exactly once.
pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await?;
    let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _migrations")
        .fetch_all(pool)
        .await?;

    let mut ups: Vec<_> = get_migrations()
        .into_iter()
        .filter(|m| m.kind == MigrationKind::Up)
        .collect();
    ups.sort_by_key(|m| m.version);

    for m in ups {
        if applied.contains(&m.version) {
            continue;
        }
        tracing::info!("Applying migration v{} ({})", m.version, m.description);
        sqlx::raw_sql(m.sql).execute(pool).await?;
        sqlx::query("INSERT INTO _migrations (version) VALUES (?)")
            .bind(m.version)
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// Open the SQLCipher-encrypted DB (resetting an undecryptable one), run migrations, and return
/// the FK-enforcing query pool the commands use.
pub async fn init_encrypted_db(app_dir: &Path) -> Result<SqlitePool, sqlx::Error> {
    let db_path = app_dir.join("nutler.db");
    let key_pragma = key_pragma_value(&db_key_hex(app_dir));

    if db_path.exists() && !can_decrypt(&db_path, &key_pragma).await {
        tracing::warn!("Existing database can't be decrypted — resetting to a fresh encrypted DB");
        remove_db_files(&db_path);
    }

    // Migrations run with FK enforcement OFF (some table-rebuild migrations require it); the
    // query pool below then enforces foreign keys.
    {
        let mig_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(base_opts(&db_path, &key_pragma).foreign_keys(false))
            .await?;
        run_migrations(&mig_pool).await?;
        mig_pool.close().await;
    }

    SqlitePool::connect_with(base_opts(&db_path, &key_pragma).foreign_keys(true)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn migrations_apply_all_ups_and_are_idempotent() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();

        run_migrations(&pool).await.unwrap();
        // A table from the migrations exists (and is empty on a fresh DB).
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0);

        let applied: Vec<i64> = sqlx::query_scalar("SELECT version FROM _migrations")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert!(applied.contains(&13)); // latest Up (is_dm)
        let count = applied.len();

        // Re-running is a no-op — nothing new applied.
        run_migrations(&pool).await.unwrap();
        let after: i64 = sqlx::query_scalar("SELECT count(*) FROM _migrations")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(after as usize, count);
    }

    #[tokio::test]
    async fn sqlcipher_encrypts_on_disk_and_gates_on_key() {
        let mut sfx = [0u8; 8];
        getrandom::getrandom(&mut sfx).unwrap();
        let name = format!(
            "nutler-cipher-{}.db",
            sfx.iter().map(|b| format!("{:02x}", b)).collect::<String>()
        );
        let path = std::env::temp_dir().join(name);
        remove_db_files(&path);
        let key1 = key_pragma_value(&"aa".repeat(32));
        let key2 = key_pragma_value(&"bb".repeat(32));

        // Write a row under key1, then close (checkpoints the WAL).
        {
            let pool = SqlitePool::connect_with(base_opts(&path, &key1).foreign_keys(true))
                .await
                .unwrap();
            sqlx::query("CREATE TABLE t (v TEXT)")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO t (v) VALUES ('secret-marker')")
                .execute(&pool)
                .await
                .unwrap();
            pool.close().await;
        }

        // On disk: the header isn't a plaintext SQLite DB and the value doesn't appear anywhere.
        let db_bytes = std::fs::read(&path).unwrap();
        assert!(
            !db_bytes.starts_with(b"SQLite format 3"),
            "SQLCipher DB header should be encrypted"
        );
        let mut all = db_bytes;
        if let Ok(wal) = std::fs::read(path.with_extension("db-wal")) {
            all.extend_from_slice(&wal);
        }
        assert!(
            !all.windows(b"secret-marker".len())
                .any(|w| w == b"secret-marker"),
            "plaintext value leaked to disk"
        );

        // The right key decrypts; a different key does not.
        assert!(can_decrypt(&path, &key1).await);
        assert!(!can_decrypt(&path, &key2).await);

        remove_db_files(&path);
    }
}
