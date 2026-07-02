//! Opaque attachment blob persistence.
//!
//! This module owns all blob-byte persistence for attachments. Nothing outside this module
//! may know that bytes currently live in SQLCipher chunk rows; the backend must remain
//! swappable behind this seam. Functions here content-address blobs by hex sha256, but never
//! parse, inspect, sniff, or otherwise interpret blob contents, preserving the future E2EE
//! path where stored bytes may be ciphertext.

// TODO(attachments Phase 3): remove once the wire transfer handlers call these.
#![allow(dead_code)]

use crate::error::{AppError, AppResult};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

// Storage-row size — a backend detail that must not leak above the seam (the wire-transfer
// chunk size is an independent constant; importing this there would couple the protocol to
// the storage backend).
const BLOB_CHUNK_ROW_BYTES: usize = 256 * 1024;

pub async fn store_blob(pool: &SqlitePool, bytes: &[u8]) -> AppResult<String> {
    if bytes.is_empty() {
        return Err(AppError::Validation(
            "Attachment blob cannot be empty".to_string(),
        ));
    }

    let sha256 = hex_sha256(bytes);
    if blob_exists_complete(pool, &sha256).await? {
        return Ok(sha256);
    }

    let mut tx = pool.begin().await?;
    let size = i64::try_from(bytes.len())
        .map_err(|_| AppError::Validation("Attachment blob is too large to store".to_string()))?;

    sqlx::query(
        "INSERT INTO attachment_blobs (sha256, size)
         VALUES ($1, $2)
         ON CONFLICT(sha256) DO UPDATE SET size = excluded.size",
    )
    .bind(&sha256)
    .bind(size)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM attachment_blob_chunks WHERE sha256 = $1")
        .bind(&sha256)
        .execute(&mut *tx)
        .await?;

    for (seq, chunk) in bytes.chunks(BLOB_CHUNK_ROW_BYTES).enumerate() {
        let seq = i64::try_from(seq)
            .map_err(|_| AppError::Validation("Attachment blob has too many chunks".to_string()))?;
        sqlx::query(
            "INSERT INTO attachment_blob_chunks (sha256, seq, data)
             VALUES ($1, $2, $3)",
        )
        .bind(&sha256)
        .bind(seq)
        .bind(chunk)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(sha256)
}

pub async fn blob_exists_complete(pool: &SqlitePool, sha256: &str) -> AppResult<bool> {
    let row = sqlx::query(
        "SELECT b.size AS size, COALESCE(SUM(LENGTH(c.data)), 0) AS stored
         FROM attachment_blobs b
         LEFT JOIN attachment_blob_chunks c ON c.sha256 = b.sha256
         WHERE b.sha256 = $1
         GROUP BY b.sha256, b.size",
    )
    .bind(sha256)
    .fetch_optional(pool)
    .await?;

    Ok(row
        .map(|row| row.get::<i64, _>("size") == row.get::<i64, _>("stored"))
        .unwrap_or(false))
}

pub async fn read_blob(pool: &SqlitePool, sha256: &str) -> AppResult<Option<Vec<u8>>> {
    // One transaction so the size lookup and chunk fetch see the same snapshot — otherwise a
    // concurrent orphan GC between the two statements would surface as a bogus "corrupt" error
    // instead of Ok(None).
    let mut tx = pool.begin().await?;
    let Some(size) =
        sqlx::query_scalar::<_, i64>("SELECT size FROM attachment_blobs WHERE sha256 = $1")
            .bind(sha256)
            .fetch_optional(&mut *tx)
            .await?
    else {
        return Ok(None);
    };

    let expected = usize::try_from(size)
        .map_err(|_| AppError::Db(format!("Blob {sha256} has invalid negative size")))?;
    let rows = sqlx::query(
        "SELECT data
         FROM attachment_blob_chunks
         WHERE sha256 = $1
         ORDER BY seq ASC",
    )
    .bind(sha256)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut out = Vec::with_capacity(expected);
    for row in rows {
        let chunk: Vec<u8> = row.get("data");
        out.extend_from_slice(&chunk);
    }

    if out.len() != expected {
        return Err(AppError::Db(format!(
            "Blob {sha256} is corrupt: expected {expected} bytes, found {}",
            out.len()
        )));
    }

    Ok(Some(out))
}

pub async fn delete_orphan_blobs(
    pool: &SqlitePool,
    older_than_secs: Option<i64>,
) -> AppResult<u64> {
    if older_than_secs.is_some_and(|secs| secs < 0) {
        return Err(AppError::Validation(
            "Blob orphan age must be non-negative".to_string(),
        ));
    }

    let mut tx = pool.begin().await?;
    let rows = if let Some(secs) = older_than_secs {
        sqlx::query(
            "SELECT b.sha256
             FROM attachment_blobs b
             WHERE NOT EXISTS (
                 SELECT 1 FROM attachments a WHERE a.sha256 = b.sha256
             )
               AND b.created_at <= datetime('now', '-' || $1 || ' seconds')",
        )
        .bind(secs)
        .fetch_all(&mut *tx)
        .await?
    } else {
        sqlx::query(
            "SELECT b.sha256
             FROM attachment_blobs b
             WHERE NOT EXISTS (
                 SELECT 1 FROM attachments a WHERE a.sha256 = b.sha256
             )",
        )
        .fetch_all(&mut *tx)
        .await?
    };

    let sha256s: Vec<String> = rows
        .into_iter()
        .map(|row| row.get::<String, _>("sha256"))
        .collect();

    let mut deleted = 0;
    for sha256 in sha256s {
        sqlx::query("DELETE FROM attachment_blob_chunks WHERE sha256 = $1")
            .bind(&sha256)
            .execute(&mut *tx)
            .await?;
        deleted += sqlx::query("DELETE FROM attachment_blobs WHERE sha256 = $1")
            .bind(&sha256)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    tx.commit().await?;

    Ok(deleted)
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_queries::{delete_attachments_for_message, insert_attachments_for_message};
    use crate::sockets::AttachmentRef;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn setup() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("open in-memory db");

        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");

        sqlx::raw_sql("PRAGMA foreign_keys=ON;")
            .execute(&pool)
            .await
            .expect("enable foreign keys");

        pool
    }

    fn patterned_bytes(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn store_blob_rejects_empty_input() {
        let pool = setup().await;
        let err = store_blob(&pool, &[]).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn store_and_read_round_trip_multi_chunk_blob() {
        let pool = setup().await;
        let bytes = patterned_bytes(600 * 1024);

        let sha256 = store_blob(&pool, &bytes).await.unwrap();
        assert_eq!(sha256, hex_sha256(&bytes));

        let chunk_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blob_chunks WHERE sha256 = $1")
                .bind(&sha256)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(chunk_count, 3);

        let read = read_blob(&pool, &sha256).await.unwrap().unwrap();
        assert_eq!(read, bytes);
    }

    #[tokio::test]
    async fn store_blob_deduplicates_complete_existing_blob() {
        let pool = setup().await;
        let bytes = patterned_bytes(128 * 1024);

        let first = store_blob(&pool, &bytes).await.unwrap();
        let second = store_blob(&pool, &bytes).await.unwrap();
        assert_eq!(first, second);

        let blob_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blobs WHERE sha256 = $1")
                .bind(&first)
                .fetch_one(&pool)
                .await
                .unwrap();
        let chunk_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blob_chunks WHERE sha256 = $1")
                .bind(&first)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(blob_count, 1);
        assert_eq!(chunk_count, 1);
        assert!(blob_exists_complete(&pool, &first).await.unwrap());
    }

    #[tokio::test]
    async fn blob_exists_complete_detects_unknown_and_missing_chunk() {
        let pool = setup().await;
        let sha256 = store_blob(&pool, &patterned_bytes(600 * 1024))
            .await
            .unwrap();

        assert!(blob_exists_complete(&pool, &sha256).await.unwrap());
        assert!(!blob_exists_complete(&pool, "missing").await.unwrap());

        sqlx::query("DELETE FROM attachment_blob_chunks WHERE sha256 = $1 AND seq = 1")
            .bind(&sha256)
            .execute(&pool)
            .await
            .unwrap();

        assert!(!blob_exists_complete(&pool, &sha256).await.unwrap());
    }

    #[tokio::test]
    async fn read_blob_returns_none_for_unknown_and_db_error_for_corrupt_blob() {
        let pool = setup().await;
        assert!(read_blob(&pool, "missing").await.unwrap().is_none());

        let sha256 = store_blob(&pool, &patterned_bytes(600 * 1024))
            .await
            .unwrap();
        sqlx::query("DELETE FROM attachment_blob_chunks WHERE sha256 = $1 AND seq = 1")
            .bind(&sha256)
            .execute(&pool)
            .await
            .unwrap();

        let err = read_blob(&pool, &sha256).await.unwrap_err();
        assert!(matches!(err, AppError::Db(_)));
    }

    #[tokio::test]
    async fn delete_orphan_blobs_respects_references_and_age_gate() {
        let pool = setup().await;
        let referenced = store_blob(&pool, b"referenced").await.unwrap();
        let orphan = store_blob(&pool, b"orphan").await.unwrap();

        let refs = vec![AttachmentRef {
            id: "att-1".to_string(),
            sha256: referenced.clone(),
            name: "ref.txt".to_string(),
            mime: "text/plain".to_string(),
            size: 10,
            width: None,
            height: None,
        }];
        insert_attachments_for_message(&pool, "message-1", &refs)
            .await
            .unwrap();

        assert_eq!(delete_orphan_blobs(&pool, None).await.unwrap(), 1);
        assert!(blob_exists_complete(&pool, &referenced).await.unwrap());
        assert!(!blob_exists_complete(&pool, &orphan).await.unwrap());
        let orphan_chunks: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM attachment_blob_chunks WHERE sha256 = $1")
                .bind(&orphan)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(orphan_chunks, 0);

        assert_eq!(
            delete_attachments_for_message(&pool, "message-1")
                .await
                .unwrap(),
            1
        );
        assert_eq!(delete_orphan_blobs(&pool, None).await.unwrap(), 1);
        assert!(!blob_exists_complete(&pool, &referenced).await.unwrap());

        let fresh = store_blob(&pool, b"fresh").await.unwrap();
        let old = store_blob(&pool, b"old").await.unwrap();
        sqlx::query(
            "UPDATE attachment_blobs
             SET created_at = datetime('now', '-2 hours')
             WHERE sha256 = $1",
        )
        .bind(&old)
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(delete_orphan_blobs(&pool, Some(3600)).await.unwrap(), 1);
        assert!(blob_exists_complete(&pool, &fresh).await.unwrap());
        assert!(!blob_exists_complete(&pool, &old).await.unwrap());
    }
}
