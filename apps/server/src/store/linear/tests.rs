use super::*;

#[test]
fn linear_cache_survives_reopen_and_old_revisions_cannot_overwrite_new_config() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("relay.db");
    let store = Store::open(&path).unwrap();
    let user = store.create_user("member", "member").unwrap();
    let inbox = Inbox {
        filter: Some(Filter {
            label: "dunlin".into(),
            team: None,
        }),
        ..Inbox::default()
    };
    store
        .linear_save(&user.id, "linear-first", "first", &inbox)
        .unwrap();
    store
        .linear_save(&user.id, "linear-second", "second", &inbox)
        .unwrap();
    assert!(
        !store
            .linear_update(&user.id, "first", &Inbox::default())
            .unwrap()
    );
    assert_eq!(store.linear_cleanup().unwrap(), vec!["linear-first"]);
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store
            .linear_connection(&user.id)
            .unwrap()
            .unwrap()
            .filter
            .label,
        "dunlin"
    );
    assert!(store.linear_due().unwrap().is_empty());
    store
        .connection
        .execute("UPDATE linear_integrations SET attempted_at=0", [])
        .unwrap();
    assert_eq!(store.linear_due().unwrap(), vec![user.id.clone()]);
    store.linear_remove(&user.id).unwrap();
    assert!(store.linear_connection(&user.id).unwrap().is_none());
    assert_eq!(store.linear_cleanup().unwrap().len(), 2);
}
