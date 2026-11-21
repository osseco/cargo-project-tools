use super::*;

#[test]
fn records_carry_kind_and_version() {
    let record = Record::TargetMissing {
        tool: "cargo-deny".into(),
        file: "project-tools.toml".into(),
        targets: vec!["x86_64-unknown-linux-gnu".into()],
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"target-missing","tool":"cargo-deny","file":"project-tools.toml","targets":["x86_64-unknown-linux-gnu"]}"#
    );
}

#[test]
fn review_pending_names_its_hash_by_kind() {
    let record = Record::ReviewPending {
        tool: "t".into(),
        file: ".project-tools/crates/t/build-review.toml".into(),
        item: ReviewItem {
            kind: "proc-macro".into(),
            package: "serde_derive".into(),
            version: "1.0.0".into(),
            path: String::new(),
            sha256: None,
            checksum: Some("cc".into()),
            commit: None,
            location: ".project-tools/targets/t/vendor/serde_derive-1.0.0".into(),
            previous: Some(Previous {
                action: "approve".into(),
                sha256: None,
                checksum: Some("bb".into()),
            }),
            known: None,
        },
        entry: "[[proc-macro]]\n".into(),
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"review-pending","tool":"t","file":".project-tools/crates/t/build-review.toml","item":{"type":"proc-macro","package":"serde_derive","version":"1.0.0","checksum":"cc","location":".project-tools/targets/t/vendor/serde_derive-1.0.0","previous":{"action":"approve","checksum":"bb"}},"entry":"[[proc-macro]]\n"}"#
    );
}

#[test]
fn advisory_leaves_out_a_missing_fix() {
    let record = Record::Advisory {
        tool: "t".into(),
        file: ".project-tools/crates/t/Cargo.lock".into(),
        id: "RUSTSEC-2026-0001".into(),
        kind: "unmaintained".into(),
        package: "p".into(),
        package_version: "1.0.0".into(),
        title: "gone".into(),
        patched: Vec::new(),
        severity: Severity::Warning,
        fix: None,
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"advisory","tool":"t","file":".project-tools/crates/t/Cargo.lock","id":"RUSTSEC-2026-0001","type":"unmaintained","package":"p","package_version":"1.0.0","title":"gone","patched":[],"severity":"warning"}"#
    );
}

#[test]
fn control_characters_are_escaped() {
    let record = Record::Error {
        message: "a\nb\u{1b}[2J".into(),
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"error","message":"a\nb\u001b[2J"}"#
    );
}

#[test]
fn held_lines_wait_for_the_screen_to_close() {
    {
        let _hold = hold();
        say!("kept {}", 1);
        note("note: kept too".into());
        assert_eq!(
            HELD.with_borrow(|h| h.clone()),
            [
                (false, "kept 1".to_string()),
                (true, "note: kept too".to_string())
            ]
        );
    }
    assert!(!holding());
    assert!(HELD.with_borrow(|h| h.is_empty()));
}

#[test]
fn a_held_command_failure_carries_its_stderr() {
    let _hold = hold();
    let err = crate::process::run(crate::process::cargo().arg("no-such-command-here"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no such command"), "{err}");
}
