use super::*;

fn identity() -> CheckpointIdentity {
    CheckpointIdentity {
        world_id: "grand-test".into(),
        manifest_fingerprint: 123,
        content_version: 1,
    }
}

fn record(owner: &str, key: &str, bytes: &[u8]) -> RuntimeResult<OwnerRecord> {
    Ok(OwnerRecord {
        owner: owner.into(),
        key: key.into(),
        format: "test-v1".into(),
        bytes: bytes.to_vec(),
    })
}

#[test]
fn session_atomic_records_span_owners_and_replace_complete_snapshot() {
    let temp = TempRoot::new();
    let store =
        SessionCheckpointStore::new(temp.child("slot"), identity(), CheckpointLimits::default())
            .expect("store");
    assert!(store.load().expect("absent").is_none());
    let first = store
        .commit(
            None,
            [
                record("world", "chunk-1", b"terrain"),
                record("gameplay", "actors", b"actors1"),
            ],
            &CancellationToken::default(),
        )
        .expect("first");
    assert_eq!(first.records().len(), 2);
    assert_eq!(
        first.records().first().expect("canonical").owner,
        "gameplay"
    );
    let loaded = store.load().expect("load").expect("head");
    assert_eq!(loaded.token(), first.token());
    loaded
        .verify_all(&CancellationToken::default())
        .expect("all bytes valid");
    assert_eq!(
        loaded.record("world", "chunk-1", "test-v1").expect("world"),
        Some(b"terrain".to_vec())
    );
    assert!(loaded.record("world", "chunk-1", "test-v2").is_err());
    assert_eq!(
        loaded
            .record("world", "missing", "test-v1")
            .expect("missing"),
        None
    );
    let second = store
        .commit(
            Some(first.token()),
            [record("gameplay", "actors", b"actors2")],
            &CancellationToken::default(),
        )
        .expect("replace");
    assert_eq!(second.token().generation, 2);
    assert_eq!(
        second
            .record("world", "chunk-1", "test-v1")
            .expect("removed"),
        None
    );
    assert_eq!(
        first
            .record("gameplay", "actors", "test-v1")
            .expect("old snapshot still usable"),
        Some(b"actors1".to_vec())
    );
}

#[test]
fn session_interrupted_export_cancel_lock_and_stale_writer_preserve_old_head() {
    let temp = TempRoot::new();
    let slot = temp.child("slot");
    let store =
        SessionCheckpointStore::new(&slot, identity(), CheckpointLimits::default()).expect("store");
    let first = store
        .commit(
            None,
            [record("gameplay", "actors", b"first")],
            &CancellationToken::default(),
        )
        .expect("first");
    let old_head = fs::read(slot.join("session.ron")).expect("head bytes");
    let interrupted = [
        record("world", "new-terrain", b"new"),
        Err(RuntimeError {
            kind: ErrorKind::Io,
            message: "simulated interruption after terrain export".into(),
        }),
    ];
    assert!(store
        .commit(
            Some(first.token()),
            interrupted,
            &CancellationToken::default()
        )
        .is_err());
    assert_eq!(
        fs::read(slot.join("session.ron")).expect("old durable head"),
        old_head
    );
    let cancelled = CancellationToken::default();
    let records = [record("world", "chunk", b"orphan")]
        .into_iter()
        .inspect(|_| cancelled.cancel());
    assert_eq!(
        store
            .commit(Some(first.token()), records, &cancelled)
            .expect_err("cancelled")
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        store.load().expect("load").expect("head").token(),
        first.token()
    );

    let writer = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(slot.join("writer.lock"))
        .expect("lock file");
    writer.lock().expect("external lock");
    assert_eq!(
        store
            .commit(
                Some(first.token()),
                [record("world", "chunk", b"locked")],
                &CancellationToken::default()
            )
            .expect_err("locked")
            .kind,
        ErrorKind::Conflict
    );
    writer.unlock().expect("unlock");
    let second = store
        .commit(
            Some(first.token()),
            [record("gameplay", "actors", b"second")],
            &CancellationToken::default(),
        )
        .expect("second");
    assert_eq!(
        store
            .commit(
                Some(first.token()),
                [record("gameplay", "actors", b"stale")],
                &CancellationToken::default()
            )
            .expect_err("stale")
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        store.load().expect("load").expect("head").token(),
        second.token()
    );
}

#[test]
fn session_partitioned_payload_exceeds_old_attachment_batch_without_aggregate_buffer() {
    let temp = TempRoot::new();
    let store =
        SessionCheckpointStore::new(temp.child("slot"), identity(), CheckpointLimits::default())
            .expect("store");
    let records = (0..65).map(|index| {
        record(
            "world",
            &format!("partition-{index:03}"),
            &vec![42; 132 * 1024],
        )
    });
    let checkpoint = store
        .commit(None, records, &CancellationToken::default())
        .expect("more than 64 records and 8MiB total");
    assert_eq!(checkpoint.records().len(), 65);
    assert!(checkpoint.records().iter().map(|r| r.bytes).sum::<usize>() > 8 * 1024 * 1024);
    assert_eq!(
        checkpoint
            .record("world", "partition-064", "test-v1")
            .expect("last partition")
            .expect("bytes")
            .len(),
        132 * 1024
    );
}

#[test]
fn session_rejects_source_format_and_body_corruption_without_empty_fallback() {
    let temp = TempRoot::new();
    let slot = temp.child("slot");
    let store =
        SessionCheckpointStore::new(&slot, identity(), CheckpointLimits::default()).expect("store");
    let snapshot = store
        .commit(
            None,
            [record("world", "damage", b"partial-hp")],
            &CancellationToken::default(),
        )
        .expect("save");
    let mut incompatible = identity();
    incompatible.manifest_fingerprint += 1;
    assert_eq!(
        SessionCheckpointStore::new(&slot, incompatible, CheckpointLimits::default())
            .expect("store")
            .load()
            .expect_err("changed source")
            .kind,
        ErrorKind::Conflict
    );
    let mut incompatible = identity();
    incompatible.content_version += 1;
    assert!(
        SessionCheckpointStore::new(&slot, incompatible, CheckpointLimits::default())
            .expect("store")
            .load()
            .is_err()
    );
    let descriptor = snapshot.records().first().expect("damage descriptor");
    let body = slot.join(format!(
        "session-records/{:016x}.bin",
        descriptor.fingerprint
    ));
    fs::write(body, b"different!").expect("tamper body");
    assert!(snapshot.verify_all(&CancellationToken::default()).is_err());
    assert!(snapshot.record("world", "damage", "test-v1").is_err());
    assert!(store
        .commit(
            Some(snapshot.token()),
            [record("world", "damage", b"partial-hp")],
            &CancellationToken::default()
        )
        .is_err());
    assert_eq!(
        store
            .load()
            .expect("intact metadata")
            .expect("head")
            .token(),
        snapshot.token()
    );
}

#[test]
fn session_record_count_bytes_duplicates_and_metadata_are_bounded() {
    let temp = TempRoot::new();
    let limits = CheckpointLimits {
        max_records: 2,
        max_record_bytes: 5,
        max_total_bytes: 8,
        ..Default::default()
    };
    let store = SessionCheckpointStore::new(temp.child("slot"), identity(), limits).expect("store");
    let first = store
        .commit(
            None,
            [record("world", "a", b"ok")],
            &CancellationToken::default(),
        )
        .expect("first");
    for records in [
        vec![record("world", "a", b"123456")],
        vec![
            record("world", "a", b"12345"),
            record("world", "b", b"12345"),
        ],
        vec![
            record("world", "a", b"1"),
            record("world", "b", b"2"),
            record("world", "c", b"3"),
        ],
        vec![record("world", "a", b"1"), record("world", "a", b"2")],
    ] {
        assert!(store
            .commit(Some(first.token()), records, &CancellationToken::default())
            .is_err());
        assert_eq!(
            store.load().expect("load").expect("head").token(),
            first.token()
        );
    }
    let tiny = SessionCheckpointStore::new(
        temp.child("tiny"),
        identity(),
        CheckpointLimits {
            max_head_bytes: 32,
            ..Default::default()
        },
    )
    .expect("tiny");
    assert_eq!(
        tiny.commit(
            None,
            [record("world", "a", b"x")],
            &CancellationToken::default()
        )
        .expect_err("head limit")
        .kind,
        ErrorKind::Limit
    );
    assert!(tiny.load().expect("no partial head").is_none());
}

#[test]
#[cfg(unix)]
fn session_content_directory_symlink_cannot_write_outside_slot() {
    let temp = TempRoot::new();
    let slot = temp.child("slot");
    let outside = temp.child("outside");
    fs::create_dir(&slot).expect("slot");
    fs::create_dir(&outside).expect("outside");
    std::os::unix::fs::symlink(&outside, slot.join("session-records")).expect("symlink");
    let store =
        SessionCheckpointStore::new(&slot, identity(), CheckpointLimits::default()).expect("store");
    assert!(store
        .commit(
            None,
            [record("world", "a", b"x")],
            &CancellationToken::default()
        )
        .is_err());
    assert_eq!(fs::read_dir(outside).expect("outside").count(), 0);
    assert!(store.load().expect("no head").is_none());
}
