//! Dialogs `cluster --yolo` may answer on a blocked first agent.
//!
//! Each entry is a detected agent kind, a bundled manifest rule id and the keys that
//! accept the option the dialog highlights by default. An entry with a gate is answered
//! only while the detection screen shows that text. Rules whose default option is not
//! a known approve or continue stay out, so their dialogs are only logged:
//! - Claude question forms are Herdr-only, gated by their select/review footer.
//!   Agent mode leaves these to Claude's own `askUserQuestionTimeout`.
//! - Codex question forms are Herdr-only, gated by their submit-answer footer.
//!   Command approvals sharing the same rule remain unanswered.
//! - claude `dynamic_workflow_prompt` and `mcp_elicitation_prompt`: no known default.
//! - codex `osc_title_blocked` and `weak_blocker` match any dialog.
//! - codex `startup_update` may default to "Update now".
//!
//! Accepted limits:
//! - Enter picks the highlighted answer, usually the first.
//! - A Claude plan approval is answered only with an `enter to select` footer;
//!   this has not been verified live.
//! - An agent whose state comes from full lifecycle hook authority, such as opencode
//!   with the herdr plugin, gets `matched_rule: null` from `agent.explain`, so its
//!   dialogs are never answered.
//! - Entries are checked against the bundled manifests only. A local override or a
//!   newer remote catalog can change what a rule id matches.
//! - opencode `permission_required` also matches opencode question dialogs; there
//!   Enter picks the highlighted, first answer.

use super::YoloMode;

/// How the watcher answers one known dialog.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Answer {
    pub(super) keys: &'static [&'static str],
    /// Any one must appear on the detection screen before each press, ignoring case.
    /// Empty means no gate.
    pub(super) gates: &'static [&'static str],
}

static ANSWERS: [(&str, &str, Answer); 4] = [
    // claude.toml: Enter submits the highlighted option. Claude rests on `1. Yes`
    // (see the comment at this rule, #2650), and no option exits.
    (
        "claude",
        "bash_permission_prompt",
        Answer {
            keys: &["Enter"],
            gates: &[],
        },
    ),
    // claude.toml: Enter submits the highlighted option. Claude rests on `1. Yes`,
    // and no option (Yes, Yes and don't ask again, No) exits.
    (
        "claude",
        "generic_permission_prompt",
        Answer {
            keys: &["Enter"],
            gates: &[],
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
            gates: &["› 1. Trust and continue"],
        },
    ),
    // opencode.toml: opencode v2.0.3 opens this prompt on `selected: "once"`, "Allow once".
    (
        "opencode",
        "permission_required",
        Answer {
            keys: &["Enter"],
            gates: &[],
        },
    ),
];

/// `--yolo-herdr` only: question forms, where Enter picks the highlighted answer.
static HERDR_ANSWERS: [(&str, &str, Answer); 4] = [
    // Footer: `Enter to select · ↑/↓ to navigate · Esc to cancel`.
    // Captured live in the qtimeout E2E and this task's E2E.
    (
        "claude",
        "live_blocked_form",
        Answer {
            keys: &["Enter"],
            gates: &["enter to select"],
        },
    ),
    // claude.toml: the named-session regex matches the same select footer.
    (
        "claude",
        "live_blocked_form_named_session",
        Answer {
            keys: &["Enter"],
            gates: &["enter to select"],
        },
    ),
    // claude.toml: `review your answers` marks the multi-question review; not captured live.
    (
        "claude",
        "legacy_no_prompt_blocker",
        Answer {
            keys: &["Enter"],
            gates: &["review your answers"],
        },
    ),
    // codex.toml: question footers gate Enter, excluding `allow command?`; not captured live.
    (
        "codex",
        "live_strong_blocker",
        Answer {
            keys: &["Enter"],
            gates: &["enter to submit answer", "enter to submit all"],
        },
    ),
];

/// How to answer a known dialog, or `None` when it must not be answered.
pub(super) fn get_answer(kind: &str, rule: &str, mode: YoloMode) -> Option<&'static Answer> {
    ANSWERS
        .iter()
        .chain(HERDR_ANSWERS.iter().filter(|_| mode == YoloMode::Herdr))
        .find(|(answer_kind, answer_rule, _)| *answer_kind == kind && *answer_rule == rule)
        .map(|(_, _, answer)| answer)
}

#[cfg(test)]
#[path = "yolo_policy_tests.rs"]
mod tests;
