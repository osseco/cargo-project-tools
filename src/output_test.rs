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
        affected: None,
    };
    assert_eq!(
        line(&record),
        r#"{"version":1,"kind":"advisory","tool":"t","file":".project-tools/crates/t/Cargo.lock","id":"RUSTSEC-2026-0001","type":"unmaintained","package":"p","package_version":"1.0.0","title":"gone","patched":[],"severity":"warning"}"#
    );
}

#[test]
fn control_characters_are_escaped() {
    let record = Record::Error {
        tool: None,
        tool_version: None,
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
        tool: None,
        tool_version: None,
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

#[test]
fn sandbox_records_name_the_tool_and_setting() {
    let missing = Record::SandboxMissing {
        tool: "t".into(),
        file: "project-tools.toml".into(),
        line: Some(5),
        message: "no build sandbox".into(),
    };
    assert_eq!(
        line(&missing),
        r#"{"version":1,"kind":"sandbox-missing","tool":"t","file":"project-tools.toml","line":5,"message":"no build sandbox"}"#
    );
    let built = Record::Built {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
        sandbox: "sandbox-exec".into(),
        bins: vec!["t".into()],
    };
    assert_eq!(
        line(&built),
        r#"{"version":1,"kind":"built","tool":"t","tool_version":"1.0.0","sandbox":"sandbox-exec","bins":["t"]}"#
    );
    let up_to_date = Record::UpToDate {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
        bins: vec!["t".into()],
    };
    assert_eq!(
        line(&up_to_date),
        r#"{"version":1,"kind":"up-to-date","tool":"t","tool_version":"1.0.0","bins":["t"]}"#
    );
}

#[test]
fn lock_changes_leave_out_the_missing_side() {
    let added = Record::LockChange {
        tool: "t".into(),
        file: ".project-tools/crates/t/Cargo.lock".into(),
        package: "zstd-safe".into(),
        from: Some("7.2.4".into()),
        to: Some("7.4.1".into()),
    };
    assert_eq!(
        line(&added),
        r#"{"version":1,"kind":"lock-change","tool":"t","file":".project-tools/crates/t/Cargo.lock","package":"zstd-safe","from":"7.2.4","to":"7.4.1"}"#
    );
    let removed = Record::LockChange {
        tool: "t".into(),
        file: "f".into(),
        package: "gone".into(),
        from: Some("1.0.0".into()),
        to: None,
    };
    assert!(!line(&removed).contains("\"to\""));
}

#[test]
fn affected_lists_only_what_the_database_names() {
    let a = Affected {
        os: vec!["windows".into()],
        ..Affected::default()
    };
    assert_eq!(serde_json::to_string(&a).unwrap(), r#"{"os":["windows"]}"#);
}

#[test]
fn the_summary_says_whether_anything_was_audited() {
    // Not audited: no counts, rather than a 0 that reads as clean.
    let summary = Record::Summary {
        ok: true,
        audited: false,
        vulnerabilities: None,
        warnings: None,
        pending_reviews: 0,
        missing_targets: 0,
    };
    assert_eq!(
        line(&summary),
        r#"{"version":1,"kind":"summary","ok":true,"audited":false,"pending_reviews":0,"missing_targets":0}"#
    );
    let summary = Record::Summary {
        ok: false,
        audited: true,
        vulnerabilities: Some(1),
        warnings: Some(0),
        pending_reviews: 0,
        missing_targets: 0,
    };
    assert_eq!(
        line(&summary),
        r#"{"version":1,"kind":"summary","ok":false,"audited":true,"vulnerabilities":1,"warnings":0,"pending_reviews":0,"missing_targets":0}"#
    );
}

#[test]
fn prepared_and_failed_builds_have_records() {
    let prepared = Record::Prepared {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
    };
    assert_eq!(
        line(&prepared),
        r#"{"version":1,"kind":"prepared","tool":"t","tool_version":"1.0.0"}"#
    );
    let failed = Record::BuildFailed {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
        sandbox: "bwrap".into(),
        exit_status: Some(101),
        prepared: true,
        command: "cargo install".into(),
        message: "m".into(),
    };
    assert_eq!(
        line(&failed),
        r#"{"version":1,"kind":"build-failed","tool":"t","tool_version":"1.0.0","sandbox":"bwrap","exit_status":101,"prepared":true,"command":"cargo install","message":"m"}"#
    );
    // Ended by a signal: no exit status.
    let signalled = Record::BuildFailed {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
        sandbox: "none".into(),
        exit_status: None,
        prepared: false,
        command: "cargo install".into(),
        message: "m".into(),
    };
    assert_eq!(
        line(&signalled),
        r#"{"version":1,"kind":"build-failed","tool":"t","tool_version":"1.0.0","sandbox":"none","prepared":false,"command":"cargo install","message":"m"}"#
    );
}

#[test]
fn an_error_about_one_tool_names_it() {
    let e = anyhow::anyhow!("isn't prepared").context(ForTool {
        tool: "t".into(),
        tool_version: "1.0.0".into(),
    });
    assert_eq!(
        line(&error_record(&e)),
        r#"{"version":1,"kind":"error","tool":"t","tool_version":"1.0.0","message":"t 1.0.0: isn't prepared"}"#
    );
    let e = anyhow::anyhow!("check failed (t)");
    assert_eq!(
        line(&error_record(&e)),
        r#"{"version":1,"kind":"error","message":"check failed (t)"}"#
    );
}
