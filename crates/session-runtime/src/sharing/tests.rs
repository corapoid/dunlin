use super::*;
use crate::tests::{context, setup};
use dunlin_domain::collaboration::WorkspaceRole;
fn owner() -> AccessContext {
    let mut a = context();
    a.project_grant
        .as_mut()
        .unwrap()
        .permissions
        .insert(Permission::ShareOwnSession);
    a
}
fn reader(id: &str) -> AccessContext {
    let mut a = context();
    a.membership.user_id = id.into();
    a.membership.role = WorkspaceRole::Viewer;
    let g = a.project_grant.as_mut().unwrap();
    g.user_id = id.into();
    g.permissions = BTreeSet::from([Permission::ViewProject, Permission::ViewSharedSession]);
    a
}
fn users(c: &Connection) {
    c.execute_batch("INSERT INTO users VALUES('reader');INSERT INTO users VALUES('other');")
        .unwrap();
}
#[test]
fn private_is_default_shared_read_covers_all_surfaces_and_never_mutates() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let plain = Journal::new(&c);
    let s = plain.create(&a, "workspace", "project").unwrap();
    plain.enqueue(&a, &s.id, "one", "private prompt").unwrap();
    let mut admin = reader("other");
    admin.membership.role = WorkspaceRole::Admin;
    assert!(plain.snapshot(&admin, &s.id).is_err());
    assert!(plain.snapshot(&r, &s.id).is_err());
    let state = plain
        .replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
        .unwrap();
    assert_eq!(state.revision, 1);
    assert!(plain.snapshot(&r, &s.id).is_err());
    let j = Journal::new(&c).validating_shared_owner(|stamp| {
        a.revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    assert_eq!(j.snapshot(&r, &s.id).unwrap().1[0].prompt, "private prompt");
    assert!(j.replay(&r, &s.id, 0).is_ok());
    assert!(j.projected_chat(&r, &s.id).is_ok());
    assert!(j.projected_actions(&r, &s.id, 0).is_ok());
    assert!(j.channel_summary(&r, &s.id).is_ok());
    assert!(j.channel_session(&r, &s.id).is_ok());
    assert!(
        j.channel_snapshots(&[ChannelSubscription {
            context: &r,
            session: &s.id,
            chat: true
        }])
        .is_ok()
    );
    assert!(
        j.channel_replay(
            &[ChannelSubscription {
                context: &r,
                session: &s.id,
                chat: true
            }],
            0
        )
        .is_ok()
    );
    let subs = [Subscription {
        context: &r,
        session: &s.id,
    }];
    let cut = j.snapshot_stream(&subs).unwrap();
    assert!(j.replay_stream(&subs, &cut.cursor).is_ok());
    let mut would_be_writer = r.clone();
    would_be_writer.membership.role = WorkspaceRole::Member;
    would_be_writer
        .project_grant
        .as_mut()
        .unwrap()
        .permissions
        .insert(Permission::CreateSession);
    assert!(
        would_be_writer
            .authorize_project("workspace", "project", Permission::CreateSession)
            .is_ok()
    );
    assert!(
        j.enqueue(
            &would_be_writer,
            &s.id,
            "write-with-valid-create-grant",
            "forbidden"
        )
        .is_err()
    );
    assert!(j.enqueue(&r, &s.id, "write", "forbidden").is_err());
    assert!(j.acquire(&r, &s.id, 0, "worker").is_err());
    assert!(j.replace_readers(&r, &s.id, 1, &[]).is_err());
    assert!(j.sharing(&r, &s.id).is_err());
    assert!(j.snapshot(&admin, &s.id).is_err());
}
#[test]
fn owner_and_recipient_revisions_revoke_and_reaffirm_requires_new_key() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let plain = Journal::new(&c);
    let s = plain.create(&a, "workspace", "project").unwrap();
    plain
        .replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
        .unwrap();
    let j = Journal::new(&c).validating_shared_owner(|stamp| {
        a.revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    let key = j.read_access_key(&r, &s.id).unwrap();
    let same = j
        .replace_readers(&a, &s.id, 1, std::slice::from_ref(&r))
        .unwrap();
    assert_eq!(same.revision, 1);
    assert_eq!(j.read_access_key(&r, &s.id).unwrap(), key);
    let mut changed_r = r.clone();
    changed_r.project_grant.as_mut().unwrap().revision += 1;
    assert!(j.snapshot(&changed_r, &s.id).is_err());
    let fresh = j
        .replace_readers(&a, &s.id, 1, std::slice::from_ref(&changed_r))
        .unwrap();
    assert_eq!(fresh.audit[0].refreshed, vec!["reader"]);
    assert_eq!(fresh.readers[0].grant_revision, 2);
    assert_ne!(j.read_access_key(&changed_r, &s.id).unwrap(), key);
    assert!(j.snapshot(&r, &s.id).is_err());
    let mut changed_owner = a.clone();
    changed_owner.membership.revision += 1;
    let newer = Journal::new(&c).validating_shared_owner(|stamp| {
        changed_owner
            .revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    assert!(newer.snapshot(&changed_r, &s.id).is_err());
    let fresh = newer
        .replace_readers(&changed_owner, &s.id, 2, std::slice::from_ref(&changed_r))
        .unwrap();
    assert_eq!(fresh.readers[0].grant_revision, 3);
    assert!(newer.snapshot(&changed_r, &s.id).is_ok());
    let mut revoked_owner = changed_owner.clone();
    revoked_owner
        .project_grant
        .as_mut()
        .unwrap()
        .permissions
        .remove(&Permission::ShareOwnSession);
    let revoked = Journal::new(&c).validating_shared_owner(|stamp| {
        revoked_owner
            .revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    assert!(revoked.snapshot(&changed_r, &s.id).is_err());
    let mut revoked_reader = changed_r.clone();
    revoked_reader
        .project_grant
        .as_mut()
        .unwrap()
        .permissions
        .remove(&Permission::ViewSharedSession);
    assert!(newer.snapshot(&revoked_reader, &s.id).is_err());
}
#[test]
fn remove_readd_cas_audit_and_reopen_preserve_aba_fencing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let c = Connection::open(&path).unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let j = Journal::new(&c).validating_shared_owner(|stamp| {
        a.revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    let s = j.create(&a, "workspace", "project").unwrap();
    let source = j.session(&s.id).unwrap().sequence;
    let clock = j.channel_clock().unwrap().sequence;
    let baseline = j.read_access_key(&a, &s.id).unwrap();
    j.replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
        .unwrap();
    let original = j.read_access_key(&r, &s.id).unwrap();
    assert!(j.replace_readers(&a, &s.id, 0, &[]).is_err());
    assert_eq!(j.sharing(&a, &s.id).unwrap().revision, 1);
    let removed = j.replace_readers(&a, &s.id, 1, &[]).unwrap();
    assert_eq!(removed.audit[0].removed, vec!["reader"]);
    assert!(j.read_access_key(&r, &s.id).is_err());
    let fresh = j
        .replace_readers(&a, &s.id, 2, std::slice::from_ref(&r))
        .unwrap();
    assert_eq!(fresh.readers[0].grant_revision, 3);
    let new_key = j.read_access_key(&r, &s.id).unwrap();
    assert_ne!(new_key, original);
    assert_eq!(j.read_access_key(&a, &s.id).unwrap(), baseline);
    assert_eq!(j.session(&s.id).unwrap().sequence, source);
    assert_eq!(j.channel_clock().unwrap().sequence, clock);
    drop(j);
    drop(c);
    let c = Connection::open(&path).unwrap();
    let j = Journal::new(&c).validating_shared_owner(|stamp| {
        a.revalidate(stamp, Permission::ShareOwnSession)
            .map_err(Error::from)
    });
    assert_eq!(j.sharing(&a, &s.id).unwrap(), fresh);
    assert_eq!(j.read_access_key(&r, &s.id).unwrap(), new_key);
    assert!(Journal::new(&c).snapshot(&r, &s.id).is_err());
}
#[test]
fn audit_insert_failure_rolls_back_readers_revision_and_invalidation() {
    use std::cell::Cell;
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let count = Cell::new(0);
    let j = Journal::new(&c).notifying(|| count.set(count.get() + 1));
    let s = j.create(&a, "workspace", "project").unwrap();
    let initial = count.get();
    c.execute_batch("CREATE TRIGGER reject_share BEFORE INSERT ON interactive_session_share_audit BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(
        j.replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
            .is_err()
    );
    let state = j.sharing(&a, &s.id).unwrap();
    assert_eq!(state.revision, 0);
    assert!(state.readers.is_empty());
    assert!(state.audit.is_empty());
    assert_eq!(count.get(), initial);
    c.execute_batch("DROP TRIGGER reject_share").unwrap();
    j.replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
        .unwrap();
    assert_eq!(count.get(), initial + 1);
    j.replace_readers(&a, &s.id, 1, std::slice::from_ref(&r))
        .unwrap();
    assert_eq!(count.get(), initial + 1);
}
#[test]
fn reader_validation_is_atomic_and_never_bypasses_domain_scope_or_human_rules() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    let mut service = r.clone();
    service.principal = PrincipalKind::Service;
    assert!(j.replace_readers(&a, &s.id, 0, &[service]).is_err());
    let mut other_scope = r.clone();
    other_scope.project_grant.as_mut().unwrap().project_id = "elsewhere".into();
    assert!(
        j.replace_readers(&a, &s.id, 0, &[r.clone(), other_scope])
            .is_err()
    );
    assert!(
        j.replace_readers(&a, &s.id, 0, &[r.clone(), r.clone()])
            .is_err()
    );
    assert!(
        j.replace_readers(&a, &s.id, 0, std::slice::from_ref(&a))
            .is_err()
    );
    let mut no_view = r.clone();
    no_view
        .project_grant
        .as_mut()
        .unwrap()
        .permissions
        .remove(&Permission::ViewProject);
    assert!(j.replace_readers(&a, &s.id, 0, &[no_view]).is_err());
    assert_eq!(j.sharing(&a, &s.id).unwrap().revision, 0);
    let mut lower = a.clone();
    lower
        .project_grant
        .as_mut()
        .unwrap()
        .permissions
        .remove(&Permission::ShareOwnSession);
    assert!(j.replace_readers(&lower, &s.id, 0, &[r]).is_err());
}
#[test]
fn full_audit_capacity_blocks_positive_authority_but_reserves_revocation() {
    let c = Connection::open_in_memory().unwrap();
    setup(&c);
    users(&c);
    let a = owner();
    let r = reader("reader");
    let other = reader("other");
    let j = Journal::new(&c);
    let s = j.create(&a, "workspace", "project").unwrap();
    j.replace_readers(&a, &s.id, 0, std::slice::from_ref(&r))
        .unwrap();
    c.execute("WITH RECURSIVE n(i) AS (SELECT 2 UNION ALL SELECT i+1 FROM n WHERE i<1024) INSERT INTO interactive_session_share_audit(session,revision,actor_id,added,removed,refreshed) SELECT ?1,i,'owner','[]','[]','[]' FROM n",[&s.id]).unwrap();
    c.execute(
        "UPDATE interactive_session_sharing SET revision=1024 WHERE session=?1",
        [&s.id],
    )
    .unwrap();
    assert!(
        j.replace_readers(&a, &s.id, 1024, &[r.clone(), other])
            .is_err()
    );
    let mut refreshed = r.clone();
    refreshed.membership.revision += 1;
    assert!(j.replace_readers(&a, &s.id, 1024, &[refreshed]).is_err());
    let state = j.replace_readers(&a, &s.id, 1024, &[]).unwrap();
    assert!(state.readers.is_empty());
    assert_eq!(state.revision, 1025);
    assert_eq!(state.audit.len(), 20);
    assert_eq!(state.audit[0].removed, vec!["reader"]);
}
