use std::path::PathBuf;

use super::*;

const DAY: Duration = Duration::from_secs(86_400);
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn state(fetched_at: u64) -> State {
    State {
        fetched_at,
        ..State::default()
    }
}

#[test]
fn decide_missing() {
    assert_eq!(
        decide(None, Policy::Auto, DAY, 0).unwrap(),
        Action::Fetch { required: true }
    );
    assert!(decide(None, Policy::Offline, DAY, 0).is_err());
}

#[test]
fn decide_by_age() {
    let s = state(1_000);
    let fresh = 1_000 + DAY.as_secs();
    let old = fresh + 1;
    assert_eq!(
        decide(Some(&s), Policy::Auto, DAY, fresh).unwrap(),
        Action::Use { stale: false }
    );
    assert_eq!(
        decide(Some(&s), Policy::Auto, DAY, old).unwrap(),
        Action::Fetch { required: false }
    );
    assert_eq!(
        decide(Some(&s), Policy::Force, DAY, fresh).unwrap(),
        Action::Fetch { required: true }
    );
    assert_eq!(
        decide(Some(&s), Policy::Offline, DAY, old).unwrap(),
        Action::Use { stale: true }
    );
}

fn tarball() -> Vec<u8> {
    tarball_with(&["RUSTSEC-2020-0001"])
}

/// A `git archive`-like tarball: a pax global header with the commit, then the prefix
/// directory and one advisory per id.
fn tarball_with(ids: &[&str]) -> Vec<u8> {
    let mut b = tar::Builder::new(Vec::new());

    let record = format!("52 comment={COMMIT}\n");
    let mut h = tar::Header::new_ustar();
    h.set_path("pax_global_header").unwrap();
    h.set_entry_type(tar::EntryType::XGlobalHeader);
    h.set_size(record.len() as u64);
    h.set_cksum();
    b.append(&h, record.as_bytes()).unwrap();

    let mut h = tar::Header::new_ustar();
    h.set_path(format!("{PREFIX}/")).unwrap();
    h.set_entry_type(tar::EntryType::Directory);
    h.set_mode(0o755);
    h.set_mtime(1_700_000_000);
    h.set_size(0);
    h.set_cksum();
    b.append(&h, &[][..]).unwrap();

    for id in ids {
        let advisory = format!(
            "```toml\n[advisory]\nid = \"{id}\"\npackage = \"foo\"\ndate = \"2020-01-01\"\n\n[versions]\npatched = [\">= 1.0.1\"]\n```\n\n# Title\n\nDescription.\n"
        );
        let mut h = tar::Header::new_ustar();
        h.set_path(format!("{PREFIX}/crates/foo/{id}.md")).unwrap();
        h.set_mode(0o644);
        h.set_size(advisory.len() as u64);
        h.set_cksum();
        b.append(&h, advisory.as_bytes()).unwrap();
    }

    let tar = b.into_inner().unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut gz, &tar).unwrap();
    gz.finish().unwrap()
}

#[test]
fn reads_commit_and_time_from_archive() {
    let (commit, updated) = archive_info(&tarball()).unwrap();
    assert_eq!(commit.as_deref(), Some(COMMIT));
    assert_eq!(updated, Some(1_700_000_000));
}

#[test]
fn extracted_archive_opens_and_keeps_state() {
    let dir = tempfile::tempdir().unwrap();
    let db_dir = dir.path().join("advisory-db");
    registry::extract_prefixed(tar_gz(&tarball()), PREFIX, &db_dir).unwrap();
    let db = rustsec::Database::open(&db_dir).unwrap();
    assert_eq!(db.iter().count(), 1);

    assert_eq!(read_state(&db_dir), None);
    let s = State {
        etag: Some("\"abc\"".into()),
        fetched_at: 5,
        commit: Some(COMMIT.into()),
        updated_at: Some(1),
    };
    write_state(&db_dir, &s).unwrap();
    assert_eq!(read_state(&db_dir), Some(s));
}

#[test]
fn state_without_advisories_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    write_state(dir.path(), &state(5)).unwrap();
    assert_eq!(read_state(dir.path()), None);
}

#[test]
fn describes_state() {
    let s = State {
        etag: None,
        fetched_at: 1_000_000 - 7_200,
        commit: Some(COMMIT.into()),
        updated_at: Some(1_000_000 - 3 * 86_400),
    };
    assert_eq!(
        describe(&s, 1_000_000),
        "0123456789ab, updated 3d ago, checked 2h ago"
    );
}

const NOW: u64 = 1_000_000;

fn body(ids: &[&str], etag: &str) -> Result<Fetched> {
    Ok(Fetched::Body {
        bytes: tarball_with(ids),
        etag: Some(etag.into()),
    })
}

fn no_request(_: Option<&str>) -> Result<Fetched> {
    panic!("no request expected")
}

/// A project dir with a database fetched at `NOW` with etag `"v1"` and one advisory.
fn fetched() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("advisory-db");
    let db = ensure_with(&path, Policy::Auto, DAY, NOW, |etag| {
        assert_eq!(etag, None);
        body(&["RUSTSEC-2020-0001"], "\"v1\"")
    })
    .unwrap();
    assert_eq!(db.iter().count(), 1);
    (dir, path)
}

fn leftovers(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(".advisory-db-"))
        .collect()
}

#[test]
fn first_fetch_writes_state() {
    let (dir, path) = fetched();
    assert_eq!(
        read_state(&path),
        Some(State {
            etag: Some("\"v1\"".into()),
            fetched_at: NOW,
            commit: Some(COMMIT.into()),
            updated_at: Some(1_700_000_000),
        })
    );
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn fresh_database_is_used_without_a_request() {
    let (_dir, path) = fetched();
    let db = ensure_with(&path, Policy::Auto, DAY, NOW + 60, no_request).unwrap();
    assert_eq!(db.iter().count(), 1);
}

#[test]
fn stale_database_is_revalidated_with_its_etag() {
    let (_dir, path) = fetched();
    let later = NOW + DAY.as_secs() + 1;
    let db = ensure_with(&path, Policy::Auto, DAY, later, |etag| {
        assert_eq!(etag, Some("\"v1\""));
        Ok(Fetched::NotModified)
    })
    .unwrap();
    assert_eq!(db.iter().count(), 1);
    let state = read_state(&path).unwrap();
    assert_eq!(state.fetched_at, later);
    assert_eq!(state.etag.as_deref(), Some("\"v1\""));
    assert_eq!(state.commit.as_deref(), Some(COMMIT));
}

#[test]
fn force_replaces_the_database() {
    let (dir, path) = fetched();
    let db = ensure_with(&path, Policy::Force, DAY, NOW + 1, |_| {
        body(&["RUSTSEC-2020-0001", "RUSTSEC-2020-0002"], "\"v2\"")
    })
    .unwrap();
    assert_eq!(db.iter().count(), 2);
    assert_eq!(rustsec::Database::open(&path).unwrap().iter().count(), 2);
    assert_eq!(read_state(&path).unwrap().etag.as_deref(), Some("\"v2\""));
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn failed_refresh_keeps_the_old_database() {
    let (_dir, path) = fetched();
    let later = NOW + DAY.as_secs() + 1;
    let db = ensure_with(&path, Policy::Auto, DAY, later, |_| bail!("offline")).unwrap();
    assert_eq!(db.iter().count(), 1);
    // Not marked as checked, so the next run tries again.
    assert_eq!(read_state(&path).unwrap().fetched_at, NOW);
}

#[test]
fn failed_forced_refresh_is_an_error() {
    let (_dir, path) = fetched();
    assert!(ensure_with(&path, Policy::Force, DAY, NOW, |_| bail!("offline")).is_err());
}

#[test]
fn invalid_download_keeps_the_old_database() {
    let (dir, path) = fetched();
    let garbage = |_: Option<&str>| {
        Ok(Fetched::Body {
            bytes: b"not a tarball".to_vec(),
            etag: Some("\"bad\"".into()),
        })
    };
    assert!(ensure_with(&path, Policy::Force, DAY, NOW, garbage).is_err());
    let later = NOW + DAY.as_secs() + 1;
    let db = ensure_with(&path, Policy::Auto, DAY, later, garbage).unwrap();
    assert_eq!(db.iter().count(), 1);
    assert_eq!(read_state(&path).unwrap().etag.as_deref(), Some("\"v1\""));
    assert!(leftovers(dir.path()).is_empty());
}

#[test]
fn archive_without_advisories_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("advisory-db");
    assert!(ensure_with(&path, Policy::Auto, DAY, NOW, |_| body(&[], "\"empty\"")).is_err());
    assert!(!path.exists());
}

#[test]
fn offline_never_fetches() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("advisory-db");
    let err = ensure_with(&path, Policy::Offline, DAY, NOW, no_request).unwrap_err();
    assert!(format!("{err:#}").contains("--no-audit"));

    let (_dir, path) = fetched();
    let much_later = NOW + 30 * DAY.as_secs();
    let db = ensure_with(&path, Policy::Offline, DAY, much_later, no_request).unwrap();
    assert_eq!(db.iter().count(), 1);
}

#[test]
fn parallel_first_fetches_download_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("advisory-db");
    let downloads = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..3 {
            s.spawn(|| {
                ensure_with(&path, Policy::Auto, DAY, NOW, |_| {
                    downloads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    body(&["RUSTSEC-2020-0001"], "\"v1\"")
                })
                .unwrap();
            });
        }
    });
    assert_eq!(downloads.into_inner(), 1);
    assert!(leftovers(dir.path()).is_empty());
}
