//! Dialogs `cluster --yolo` may answer on a blocked first agent.
//!
//! Each entry is a detected agent kind, a bundled manifest rule id and the keys that
//! accept the option the dialog highlights by default. An entry with a gate is answered
//! only while the detection screen shows that text. Rules whose default option is not
//! a known approve or continue stay out, so their dialogs are only logged:
//! - claude `live_blocked_form*` and `legacy_no_prompt_blocker` also match question
//!   forms; Claude answers those itself after `askUserQuestionTimeout`.
//! - claude `dynamic_workflow_prompt` and `mcp_elicitation_prompt`: no known default.
//! - codex `osc_title_blocked` and `weak_blocker` match any dialog.
//! - codex `live_strong_blocker` also matches question forms and pickers.
//! - codex `startup_update` may default to "Update now".
//!
//! Accepted limits:
//! - An agent whose state comes from full lifecycle hook authority, such as opencode
//!   with the herdr plugin, gets `matched_rule: null` from `agent.explain`, so its
//!   dialogs are never answered.
//! - Entries are checked against the bundled manifests only. A local override or a
//!   newer remote catalog can change what a rule id matches.
//! - opencode `permission_required` also matches opencode question dialogs; there
//!   Enter picks the highlighted, first answer.

/// How the watcher answers one known dialog.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Answer {
    pub(super) keys: &'static [&'static str],
    /// Text the detection screen must show right before each press.
    pub(super) gate: Option<&'static str>,
}

static ANSWERS: [(&str, &str, Answer); 4] = [
    // claude.toml: Enter submits the highlighted option. Claude rests on `1. Yes`
    // (see the comment at this rule, #2650), and no option exits.
    (
        "claude",
        "bash_permission_prompt",
        Answer {
            keys: &["Enter"],
            gate: None,
        },
    ),
    // claude.toml: Enter submits the highlighted option. Claude rests on `1. Yes`,
    // and no option (Yes, Yes and don't ask again, No) exits.
    (
        "claude",
        "generic_permission_prompt",
        Answer {
            keys: &["Enter"],
            gate: None,
        },
    ),
    // codex.toml: codex 0.164 offers `1. Trust and continue` and `2. Quit`, and Enter
    // submits the highlighted one. The gate, captured live, requires the cursor on
    // option 1; it also skips the legacy dialog, whose default is unknown.
    (
        "codex",
        "trust_directory",
        Answer {
            keys: &["Enter"],
            gate: Some("› 1. Trust and continue"),
        },
    ),
    // opencode.toml: opencode v2.0.3 opens this prompt on `selected: "once"`, "Allow once".
    (
        "opencode",
        "permission_required",
        Answer {
            keys: &["Enter"],
            gate: None,
        },
    ),
];

/// How to answer a known dialog, or `None` when it must not be answered.
pub(super) fn get_answer(kind: &str, rule: &str) -> Option<&'static Answer> {
    ANSWERS
        .iter()
        .find(|(answer_kind, answer_rule, _)| *answer_kind == kind && *answer_rule == rule)
        .map(|(_, _, answer)| answer)
}

#[cfg(test)]
#[path = "yolo_policy_tests.rs"]
mod tests;
