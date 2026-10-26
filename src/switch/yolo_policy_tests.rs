use super::*;

/// The bundled manifest Herdr evaluates for a detected agent kind.
fn bundled_manifest(kind: &str) -> toml::Value {
    let text = match kind {
        "claude" => include_str!("../detect/manifests/claude.toml"),
        "codex" => include_str!("../detect/manifests/codex.toml"),
        "opencode" => include_str!("../detect/manifests/opencode.toml"),
        _ => panic!("no bundled manifest listed for {kind}"),
    };
    toml::from_str(text).unwrap()
}

/// Every `contains` string anywhere in a rule, nested gates included.
fn collect_contains(value: &toml::Value, texts: &mut Vec<String>) {
    match value {
        toml::Value::Table(table) => {
            for (key, entry) in table {
                match (key.as_str(), entry) {
                    ("contains", toml::Value::Array(items)) => texts.extend(
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned)),
                    ),
                    _ => collect_contains(entry, texts),
                }
            }
        }
        toml::Value::Array(items) => {
            for item in items {
                collect_contains(item, texts);
            }
        }
        _ => {}
    }
}

#[test]
fn every_answer_names_a_blocked_rule_in_the_bundled_manifest() {
    for (kind, rule, answer) in ANSWERS.iter().chain(&HERDR_ANSWERS) {
        let manifest = bundled_manifest(kind);
        assert_eq!(manifest["id"].as_str(), Some(*kind));
        let rules = manifest["rules"].as_array().unwrap();
        let found = rules
            .iter()
            .find(|entry| entry["id"].as_str() == Some(*rule))
            .unwrap_or_else(|| panic!("{kind} has no rule {rule}"));
        assert_eq!(found["state"].as_str(), Some("blocked"), "{kind} {rule}");
        assert!(!answer.keys.is_empty(), "{kind} {rule}");
        // A gate contains text the rule itself already looks for.
        for gate in answer.gates {
            let mut texts = Vec::new();
            collect_contains(found, &mut texts);
            // Named sessions express the select footer as a regex alternative.
            let has_regex_gate = *gate == "enter to select"
                && found
                    .get("regex")
                    .and_then(toml::Value::as_array)
                    .is_some_and(|patterns| {
                        patterns
                            .iter()
                            .filter_map(toml::Value::as_str)
                            .any(|pattern| {
                                pattern
                                    .to_lowercase()
                                    .contains(r"enter\s+to\s+(?:confirm|select")
                            })
                    });
            assert!(
                has_regex_gate
                    || texts
                        .iter()
                        .any(|text| gate.to_lowercase().contains(&text.to_lowercase())),
                "{kind} {rule} {gate} {texts:?}"
            );
        }
        assert_eq!(get_answer(kind, rule, YoloMode::Herdr), Some(answer));
    }
}

#[test]
fn question_forms_and_unlisted_dialogs_get_no_answer() {
    for (kind, rule) in [
        ("claude", "live_blocked_form"),
        ("claude", "live_blocked_form_named_session"),
        ("claude", "legacy_no_prompt_blocker"),
        ("claude", "mcp_elicitation_prompt"),
        ("codex", "live_strong_blocker"),
        ("codex", "startup_update"),
        ("codex", "osc_title_blocked"),
        ("kimi", "bash_permission_prompt"),
        ("claude", "trust_directory"),
        ("", ""),
    ] {
        assert_eq!(
            get_answer(kind, rule, YoloMode::Agent),
            None,
            "{kind} {rule}"
        );
    }
}

#[test]
fn herdr_answers_only_common_dialogs_and_gated_question_forms() {
    for (kind, rule, answer) in ANSWERS.iter().chain(&HERDR_ANSWERS) {
        assert_eq!(get_answer(kind, rule, YoloMode::Herdr), Some(answer));
        if HERDR_ANSWERS.iter().any(|(k, r, _)| k == kind && r == rule) {
            assert_eq!(get_answer(kind, rule, YoloMode::Agent), None);
        } else {
            assert_eq!(get_answer(kind, rule, YoloMode::Agent), Some(answer));
        }
    }
    for (kind, rule) in [
        ("claude", "dynamic_workflow_prompt"),
        ("claude", "mcp_elicitation_prompt"),
        ("codex", "startup_update"),
        ("codex", "osc_title_blocked"),
        ("codex", "weak_blocker"),
        ("kimi", "bash_permission_prompt"),
        ("claude", "trust_directory"),
        ("", ""),
    ] {
        assert_eq!(get_answer(kind, rule, YoloMode::Herdr), None);
    }
}
