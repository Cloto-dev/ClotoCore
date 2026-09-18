//! bug-486 regression: a corrupt / non-SQLite kernel DB file must self-heal
//! (quarantine the unreadable file aside + recreate a fresh DB) instead of
//! dead-ending startup with a fatal dialog + exit(1). Genuinely unrecoverable
//! open failures must still propagate.

use std::fs;

#[tokio::test]
async fn corrupt_db_is_quarantined_and_recreated() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cloto_memories.db");
    // A non-SQLite file — mimics the dummy DB that first triggered bug-486 on
    // the opverify apex VM run. Opening it as SQLite yields SQLITE_NOTADB.
    let garbage: &[u8] = b"this is definitely not a sqlite database";
    fs::write(&db_path, garbage).unwrap();
    let url = format!("sqlite:{}", db_path.display());

    // Self-heal: returns a working pool instead of erroring out.
    let pool = cloto_core::open_kernel_db(&url, None)
        .await
        .expect("open_kernel_db should recover from a corrupt DB");
    let one: i64 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("the freshly recreated DB should be queryable");
    assert_eq!(one, 1);

    // A fresh valid DB now exists at the original path.
    assert!(
        db_path.exists(),
        "a fresh DB must be recreated at the original path"
    );

    // The corrupt bytes were preserved in exactly one timestamped backup
    // (renamed aside, never deleted — Destructive DB rule).
    let backups: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
        .collect();
    assert_eq!(
        backups.len(),
        1,
        "the corrupt file must be quarantined to exactly one .corrupt-*.bak"
    );
    let preserved = fs::read(backups[0].path()).unwrap();
    assert_eq!(
        preserved, garbage,
        "the quarantine backup must preserve the original (corrupt) bytes verbatim"
    );
}

#[tokio::test]
async fn non_corrupt_open_failure_propagates() {
    // Pointing the DB at a directory path makes the open fail with CANTOPEN,
    // NOT the corrupt-DB class — recovery must not trigger and the error must
    // propagate so genuinely unrecoverable conditions still hard-fail.
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().display());
    let result = cloto_core::open_kernel_db(&url, None).await;
    assert!(
        result.is_err(),
        "a non-corrupt-DB open failure must propagate, not be silently recovered"
    );
    // Nothing should have been quarantined for a non-corrupt failure.
    let quarantined = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
    assert!(
        !quarantined,
        "a non-corrupt failure must not quarantine anything"
    );
}
