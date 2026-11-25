// SPDX-License-Identifier: AGPL-3.0-only

use std::fs;

use hs_core::event::Kind;
use hs_node::Node;
use serde_json::json;
use sqlx_core::connection::ConnectOptions as _;
use sqlx_core::raw_sql::raw_sql;
use sqlx_sqlite::SqliteConnectOptions;

#[tokio::test]
async fn journal_failure_does_not_advance_state_or_acknowledge_a_retry() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::init(dir.path(), "Audit", None).await.unwrap().node;
    let id = node.create_space("notes", json!({})).await.unwrap();
    let payload = b"{}";
    let proposal = node
        .propose(
            &node.owner(),
            &id,
            Kind::Put,
            "note/first",
            Some(payload),
            "application/json",
            Some(vec![1; 16]),
        )
        .unwrap();
    let tail = dir.path().join("logs").join(id.to_hex()).join("tail");
    let backup = tail.with_extension("saved");
    fs::rename(&tail, &backup).unwrap();
    fs::create_dir(&tail).unwrap();

    assert!(node.submit(&id, &proposal, Some(payload)).await.is_err());
    assert!(node.get(&id, "note/first").await.unwrap().is_none());
    assert_eq!(
        node.entries(&id, Some(0), 10).await.unwrap(),
        [] as [std::vec::Vec<u8>; 0]
    );

    fs::remove_dir(&tail).unwrap();
    fs::rename(&backup, &tail).unwrap();
    let committed = node.submit(&id, &proposal, Some(payload)).await.unwrap();
    assert_eq!(committed.0, 1);
    node.write(
        &node.owner(),
        &id,
        Kind::Put,
        "note/other",
        Some(payload),
        "application/json",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        node.submit(&id, &proposal, Some(payload)).await.unwrap(),
        committed
    );
    node.index.close().await;
    drop(node);

    let reopened = Node::open(dir.path()).await.unwrap();
    assert!(reopened.get(&id, "note/first").await.unwrap().is_some());
    assert_eq!(reopened.entries(&id, Some(0), 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn index_failure_resumes_on_retry_without_a_duplicate_entry() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::init(dir.path(), "Audit", None).await.unwrap().node;
    let space = node.create_space("notes", json!({})).await.unwrap();
    let mut db = SqliteConnectOptions::new()
        .filename(dir.path().join("index.db"))
        .connect()
        .await
        .unwrap();
    raw_sql("CREATE TRIGGER fail_index BEFORE INSERT ON resources BEGIN SELECT RAISE(FAIL, 'unavailable'); END")
        .execute(&mut db).await.unwrap();
    let proposal = node
        .propose(
            &node.owner(),
            &space,
            Kind::Put,
            "note/first",
            Some(b"{}"),
            "application/json",
            None,
        )
        .unwrap();

    assert!(node.submit(&space, &proposal, Some(b"{}")).await.is_err());
    assert_eq!(node.entries(&space, Some(0), 10).await.unwrap().len(), 1);
    assert!(
        node.index
            .get(&space.to_hex(), "note/first")
            .await
            .unwrap()
            .is_none()
    );

    raw_sql("DROP TRIGGER fail_index")
        .execute(&mut db)
        .await
        .unwrap();
    let mut events = node.subscribe();
    assert_eq!(
        node.submit(&space, &proposal, Some(b"{}")).await.unwrap().0,
        1
    );
    assert_eq!(events.try_recv().unwrap().seq, 1);
    assert!(events.try_recv().is_err());
    assert_eq!(node.entries(&space, Some(0), 10).await.unwrap().len(), 1);
    assert!(
        node.index
            .get(&space.to_hex(), "note/first")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn segment_manifest_failure_can_resume_without_rewriting_the_entry() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::init(dir.path(), "Audit", None).await.unwrap().node;
    let id = node.create_space("notes", json!({})).await.unwrap();
    let owner = node.owner();
    for _ in 1..hs_node::SEGMENT_ENTRIES - 1 {
        node.write(
            &owner,
            &id,
            Kind::Put,
            "note/main",
            Some(b"{}"),
            "application/json",
            None,
        )
        .await
        .unwrap();
    }
    let blocked = dir
        .path()
        .join("logs")
        .join(id.to_hex())
        .join("segments.txt");
    fs::create_dir(&blocked).unwrap();
    let signed = node
        .propose(
            &owner,
            &id,
            Kind::Put,
            "note/sealed",
            Some(b"{}"),
            "application/json",
            None,
        )
        .unwrap();
    assert!(node.submit(&id, &signed, Some(b"{}")).await.is_err());
    let count = hs_node::SEGMENT_ENTRIES;
    assert_eq!(
        node.entries(&id, None, count + 1).await.unwrap().len(),
        count
    );

    fs::remove_dir(&blocked).unwrap();
    let (seq, _) = node.submit(&id, &signed, Some(b"{}")).await.unwrap();
    assert_eq!(seq, count as u64 - 1);
    node.index.close().await;
    drop(node);
    let reopened = Node::open(dir.path()).await.unwrap();
    assert_eq!(
        reopened.entries(&id, None, count + 1).await.unwrap().len(),
        count
    );
    assert!(reopened.get(&id, "note/sealed").await.unwrap().is_some());
}

#[tokio::test]
async fn rejected_config_update_leaves_current_and_persisted_values_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::init(dir.path(), "Audit", None).await.unwrap().node;
    let before = node.config();
    let config = dir.path().join("config.json");
    let on_disk = fs::read(&config).unwrap();
    assert!(node.set_config("identity", json!(false)).is_err());
    assert_eq!(node.config().raw, before.raw);
    assert_eq!(fs::read(&config).unwrap(), on_disk);

    let backup = config.with_extension("saved");
    fs::rename(&config, &backup).unwrap();
    fs::create_dir(&config).unwrap();
    assert!(node.set_config("name", json!("Renamed")).is_err());
    assert_eq!(node.config().raw, before.raw);

    fs::remove_dir(&config).unwrap();
    fs::rename(&backup, &config).unwrap();
    node.set_config("name", json!("Renamed")).unwrap();
    assert_eq!(node.config().name, "Renamed");
}
