use super::*;

#[test]
fn rename_notice_only_when_the_requested_name_changed() {
    assert_eq!(rename_notice(None, None), None);
    assert_eq!(rename_notice(Some("rev"), Some("rev")), None);
    assert_eq!(
        rename_notice(Some("rev"), Some("rev-2")).as_deref(),
        Some("agent name rev is taken; using rev-2")
    );
}

#[test]
fn only_shutdown_refusals_are_retried() {
    assert!(refusal_is_retryable("server is shutting down"));
    assert!(!refusal_is_retryable(
        "passthrough spawn failed: invalid command"
    ));
}
