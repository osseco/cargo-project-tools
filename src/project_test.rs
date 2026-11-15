use super::*;

#[test]
fn gitignore_entries_are_added_once() {
    let first = with_ignored_entries("");
    for entry in IGNORED {
        assert!(first.lines().any(|l| l == *entry));
    }
    assert_eq!(with_ignored_entries(&first), first);
}

#[test]
fn gitignore_keeps_user_lines() {
    let out = with_ignored_entries("/custom\n/bin/");
    assert!(out.starts_with("/custom\n/bin/\n"));
    assert_eq!(out.matches("/bin/").count(), 1);
}
