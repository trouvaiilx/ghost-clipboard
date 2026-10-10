use tempfile::tempdir;

use ghost_clipboard::domain::capture::prepare;
use ghost_clipboard::domain::entry::EntryId;
use ghost_clipboard::domain::settings::{CaptureSettings, DedupMode, RetentionPolicy};
use ghost_clipboard::storage::repository::{HistoryQuery, RecordOutcome, Repository};

#[test]
fn durable_storage_across_restarts() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_history.db");

    let text_content = "fn main() {\r\n    println!(\"Hello World! 🦀\");\r\n}";

    // First session: write entry and pin it
    {
        let mut repo = Repository::open(&db_path).expect("failed to open repository");
        let entry = prepare(
            text_content.to_string(),
            &CaptureSettings::default(),
            Some("code.exe".to_string()),
            1_000_000,
        )
        .expect("prepare failed");

        let outcome = repo
            .record(entry, DedupMode::Anywhere)
            .expect("record failed");
        let id = match outcome {
            RecordOutcome::Inserted(id) => id,
            _ => panic!("expected inserted"),
        };

        repo.toggle_pin(id).expect("toggle pin failed");
    }

    // Second session: reopen repository from disk and check contents survive
    {
        let repo = Repository::open(&db_path).expect("failed to reopen repository");
        let list = repo.list(&HistoryQuery::default()).expect("list failed");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, EntryId(1));
        assert!(list[0].pinned);
        assert_eq!(list[0].source_app.as_deref(), Some("code.exe"));

        let full_text = repo.get_text(EntryId(1)).expect("get_text failed");
        assert_eq!(full_text.as_deref(), Some(text_content));
    }
}

#[test]
fn verbatim_text_preservation() {
    let mut repo = Repository::open_in_memory().unwrap();
    let test_cases = [
        "Single line",
        "Line 1\r\nLine 2\r\nLine 3",
        "Line 1\nLine 2\nLine 3",
        "    leading spaces and tabs\t\t",
        "Unicode: 日本語, Español (ñ, á), Русский, Arabic العربية, Emoji: 🚀🔥✨",
        "SQL injection attempt: '; DROP TABLE entries; --",
        "HTML/XML: <script>alert('xss')</script> &amp; \"quotes\"",
    ];

    for (i, case) in test_cases.iter().enumerate() {
        let entry = prepare(
            case.to_string(),
            &CaptureSettings::default(),
            None,
            i as i64 * 100,
        )
        .unwrap();

        let id = match repo.record(entry, DedupMode::Anywhere).unwrap() {
            RecordOutcome::Inserted(id) => id,
            _ => panic!(),
        };

        let loaded = repo.get_text(id).unwrap().unwrap();
        assert_eq!(
            &loaded, case,
            "Text must be stored and retrieved byte-exact"
        );
    }
}

#[test]
fn deduplication_and_ordering() {
    let mut repo = Repository::open_in_memory().unwrap();
    let settings = CaptureSettings::default();

    // 1. Insert A, B, C
    repo.record(
        prepare("A".into(), &settings, None, 100).unwrap(),
        DedupMode::Anywhere,
    )
    .unwrap();
    repo.record(
        prepare("B".into(), &settings, None, 200).unwrap(),
        DedupMode::Anywhere,
    )
    .unwrap();
    repo.record(
        prepare("C".into(), &settings, None, 300).unwrap(),
        DedupMode::Anywhere,
    )
    .unwrap();

    assert_eq!(repo.count().unwrap(), 3);

    // Initial order should be C (300), B (200), A (100)
    let list = repo.list(&HistoryQuery::default()).unwrap();
    assert_eq!(
        list.iter().map(|s| s.preview.as_str()).collect::<Vec<_>>(),
        vec!["C", "B", "A"]
    );

    // 2. Re-copy A at time 400. DedupMode::Anywhere should move A to the top
    let outcome = repo
        .record(
            prepare("A".into(), &settings, None, 400).unwrap(),
            DedupMode::Anywhere,
        )
        .unwrap();
    assert_eq!(
        outcome,
        RecordOutcome::Deduplicated {
            id: EntryId(1),
            moved_to_top: true
        }
    );
    assert_eq!(repo.count().unwrap(), 3);

    let list = repo.list(&HistoryQuery::default()).unwrap();
    assert_eq!(
        list.iter().map(|s| s.preview.as_str()).collect::<Vec<_>>(),
        vec!["A", "C", "B"]
    );
}

#[test]
fn retention_respects_pinned_entries() {
    let mut repo = Repository::open_in_memory().unwrap();
    let settings = CaptureSettings::default();

    // Add 10 entries
    let mut ids = Vec::new();
    for i in 1..=10 {
        let outcome = repo
            .record(
                prepare(format!("Entry {i}"), &settings, None, i * 100).unwrap(),
                DedupMode::Anywhere,
            )
            .unwrap();
        if let RecordOutcome::Inserted(id) = outcome {
            ids.push(id);
        }
    }

    // Pin entry 1 and 2 (the oldest)
    repo.toggle_pin(ids[0]).unwrap();
    repo.toggle_pin(ids[1]).unwrap();

    // Retention: keep at most 3 unpinned entries
    let policy = RetentionPolicy {
        max_entries: Some(3),
        max_age: None,
    };
    let deleted = repo.apply_retention(&policy, 2000).unwrap();
    assert_eq!(deleted, 5); // 8 unpinned entries - 3 kept = 5 deleted

    // Total remaining: 2 pinned + 3 unpinned = 5 entries
    assert_eq!(repo.count().unwrap(), 5);

    // Verify pinned entries still exist
    assert!(repo.get_text(ids[0]).unwrap().is_some());
    assert!(repo.get_text(ids[1]).unwrap().is_some());
}

#[test]
fn secure_delete_and_wal_truncation_on_clear() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("secure_delete_test.db");
    let wal_path = dir.path().join("secure_delete_test.db-wal");

    let canary = "SENSITIVE_CANARY_TEXT_XYZ_123456789";

    // 1. Insert canary entry
    {
        let mut repo = Repository::open(&db_path).expect("open failed");
        let entry = prepare(canary.to_string(), &CaptureSettings::default(), None, 1_000)
            .expect("prepare failed");
        repo.record(entry, DedupMode::Anywhere)
            .expect("record failed");

        // Clear all entries (which runs secure_delete and wal_checkpoint(TRUNCATE))
        let deleted = repo.clear_all().expect("clear_all failed");
        assert_eq!(deleted, 1);
    }

    // 2. Inspect raw file contents on disk
    let db_bytes = std::fs::read(&db_path).unwrap_or_default();
    let wal_bytes = if wal_path.exists() {
        std::fs::read(&wal_path).unwrap_or_default()
    } else {
        Vec::new()
    };

    let canary_bytes = canary.as_bytes();
    let in_db = db_bytes
        .windows(canary_bytes.len())
        .any(|w| w == canary_bytes);
    let in_wal = wal_bytes
        .windows(canary_bytes.len())
        .any(|w| w == canary_bytes);

    assert!(
        !in_db,
        "secure_delete must overwrite deleted text in the db file with zeroes"
    );
    assert!(
        !in_wal,
        "wal_checkpoint(TRUNCATE) must remove deleted text from the WAL file"
    );
}

#[tokio::test]
async fn test_corrupt_database_detection_and_recovery() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("corrupt_test.db");

    // 1. Write corrupted content into the database file
    std::fs::write(&db_path, b"NOT A VALID SQLITE DATABASE FILE HEADER!!!").unwrap();

    // 2. Spawn Storage should detect corruption and return StorageError::Corrupt
    let spawn_res = ghost_clipboard::storage::Storage::spawn(&db_path);
    assert!(
        matches!(
            spawn_res,
            Err(ghost_clipboard::storage::StorageError::Corrupt(_))
        ),
        "corrupted file must return StorageError::Corrupt"
    );

    // 3. Rename corrupt file using helper
    let timestamp = 1700000000;
    let corrupt_path = ghost_clipboard::storage::rename_corrupt_db(&db_path, timestamp).unwrap();
    assert!(!db_path.exists(), "original db path must be moved");
    assert!(
        corrupt_path.exists(),
        "backup file with corrupt prefix must exist"
    );

    // 4. Starting fresh database at original path should now succeed
    let storage =
        ghost_clipboard::storage::Storage::spawn(&db_path).expect("fresh spawn must succeed");
    let report = storage
        .handle()
        .report()
        .await
        .expect("fresh storage report must work");
    assert_eq!(report.total_entries, 0);
}
