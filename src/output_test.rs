use super::*;

#[test]
fn records_carry_kind_and_version() {
    let record = Record::TargetMissing {
        tool: "cargo-deny".into(),
        file: "project-tools.toml".into(),
        line: Some(4),
        targets: vec!["x86_64-unknown-linux-gnu".into()],
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"target-missing","tool":"cargo-deny","file":"project-tools.toml","line":4,"targets":["x86_64-unknown-linux-gnu"]}"#
    );
}

#[test]
fn review_pending_names_its_hash_by_kind() {
    let record = Record::ReviewPending {
        tool: "t".into(),
        file: ".project-tools/crates/t/build-review.toml".into(),
        line: Some(12),
        item: Box::new(ReviewItem {
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
                commit: None,
                changed: vec!["checksum"],
                version: None,
                location: None,
            }),
            known: None,
        }),
        spec: "serde_derive@1.0.0=cc".into(),
        entry: "[[proc-macro]]\n".into(),
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"review-pending","tool":"t","file":".project-tools/crates/t/build-review.toml","line":12,"item":{"type":"proc-macro","package":"serde_derive","version":"1.0.0","checksum":"cc","location":".project-tools/targets/t/vendor/serde_derive-1.0.0","previous":{"action":"approve","checksum":"bb","changed":["checksum"]}},"spec":"serde_derive@1.0.0=cc","entry":"[[proc-macro]]\n"}"#
    );
}

#[test]
fn advisory_leaves_out_a_missing_fix() {
    let record = Record::Advisory {
        tool: "t".into(),
        file: ".project-tools/crates/t/Cargo.lock".into(),
        line: None,
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

#[test]
fn characters_a_terminal_acts_on_or_hides_are_escaped() {
    let message = "a\u{7f}b\u{9b}2Jc\u{202e}d\u{2028}e\u{200b}f\u{e0041}g \u{e9}";
    let json = line(&Record::Error {
        message: message.into(),
    });
    assert_eq!(
        json,
        r#"{"version":1,"kind":"error","message":"a\u007fb\u009b2Jc\u202ed\u2028e\u200bf\udb40\udc41g é"}"#
    );
    // Still the same value.
    let back: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(back["message"], message);
}

#[test]
fn sfw_missing_is_a_record_of_its_own() {
    let record = Record::SfwMissing {
        message: "sfw not found on PATH; downloads are not filtered".into(),
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"sfw-missing","message":"sfw not found on PATH; downloads are not filtered"}"#
    );
}
