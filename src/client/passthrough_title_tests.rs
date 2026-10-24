use super::*;

#[test]
fn status_titles_compose_symbols_and_reuse_the_last_program_title() {
    let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
    assert_eq!(
        rewriter.status_title(),
        "\x1b]2;herdr · w1:p2\x07".as_bytes()
    );
    for icon in ["✅", "⏳", "✋", "💤"] {
        rewriter.set_status(Some(icon));
        assert_eq!(
            rewriter.status_title(),
            format!("\x1b]2;{icon} herdr · w1:p2\x07").as_bytes()
        );
    }
    let mut out = Vec::new();
    rewriter.rewrite(b"\x1b]0;first\x07\x1b]2;Fix CI\x1b\\", &mut out);
    assert!(out.ends_with("\x1b]2;💤 herdr · w1:p2 — Fix CI\x1b\\".as_bytes()));
    rewriter.set_status(Some("✅"));
    assert_eq!(
        rewriter.status_title(),
        "\x1b]2;✅ herdr · w1:p2 — Fix CI\x07".as_bytes()
    );
    rewriter.set_status(None);
    assert_eq!(
        rewriter.status_title(),
        "\x1b]2;herdr · w1:p2 — Fix CI\x07".as_bytes()
    );
    rewriter.rewrite(b"\x1b]2;\x07", &mut out);
    assert_eq!(
        rewriter.status_title(),
        "\x1b]2;herdr · w1:p2\x07".as_bytes()
    );
}

#[test]
fn title_reemission_waits_for_complete_control_sequences_and_utf8() {
    for (start, end) in [
        (b"\x1b[31".as_slice(), b"m".as_slice()),
        (b"\x1b]2;title", b"\x1b\\"),
        (b"\x1b(", b"B"),
        (b"\x1b(\x01", b"B"),
        (b"\x1b_opaque", b"\x1b\\"),
        (b"\xe2\x97", b"\x90"),
    ] {
        let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
        let mut out = Vec::new();
        assert!(rewriter.at_ground());
        rewriter.rewrite(start, &mut out);
        assert!(!rewriter.at_ground(), "{start:?}");
        rewriter.rewrite(end, &mut out);
        assert!(rewriter.at_ground(), "{end:?}");
    }
}

#[test]
fn rewrites_bel_and_st_titles_preserving_the_osc_kind() {
    for ps in [0, 2] {
        for terminator in ["\x07", "\x1b\\"] {
            let input = format!("\x1b]{ps};Fix CI{terminator}");
            let expected = format!("\x1b]{ps};herdr · w1:p2 — Fix CI{terminator}");
            let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
            let mut out = Vec::new();
            rewriter.rewrite(input.as_bytes(), &mut out);
            assert_eq!(out, expected.as_bytes());
        }
    }
}

#[test]
fn rewrites_titles_split_at_every_byte_boundary() {
    for ps in [0, 2] {
        for terminator in ["\x07", "\x1b\\"] {
            let input = format!("\x1b]{ps};◑ Fix CI on macOS{terminator}");
            let expected = format!("\x1b]{ps};herdr · w1:p2 — ◑ Fix CI on macOS{terminator}");
            for split in 0..=input.len() {
                let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
                let mut out = Vec::new();
                rewriter.rewrite(&input.as_bytes()[..split], &mut out);
                rewriter.rewrite(&input.as_bytes()[split..], &mut out);
                assert_eq!(out, expected.as_bytes(), "split at {split}");
            }
        }
    }
}

#[test]
fn rewrites_empty_titles_without_a_separator() {
    for ps in [0, 2] {
        for terminator in ["\x07", "\x1b\\"] {
            let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
            let mut out = Vec::new();
            rewriter.rewrite(format!("\x1b]{ps};{terminator}").as_bytes(), &mut out);
            assert_eq!(
                out,
                format!("\x1b]{ps};herdr · w1:p2{terminator}").as_bytes()
            );
        }
    }
}

#[test]
fn preserves_other_osc_csi_text_and_lone_esc() {
    let input = b"text\xff\x1b[31mred\x1b[0m\x1b]1;icon\x07\x1b]7;file:///tmp\x1b\\\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x07\x1b]52;c;opaque\x1b]2;literal\x07\x1b\x1b";
    for chunk_size in [1, input.len()] {
        let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
        let mut out = Vec::new();
        for chunk in input.chunks(chunk_size) {
            rewriter.rewrite(chunk, &mut out);
        }
        rewriter.finish(&mut out);
        assert_eq!(out, input);
    }
}

#[test]
fn oversized_pending_is_forwarded_until_its_terminator() {
    for terminator in [b"\x07".as_slice(), b"\x1b\\"] {
        let mut input = b"\x1b]2;".to_vec();
        input.resize(4096, b'x');
        let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
        let mut out = Vec::new();
        rewriter.rewrite(&input, &mut out);
        assert!(out.is_empty());
        input.push(b'x');
        rewriter.rewrite(b"x", &mut out);
        assert_eq!(out, input);
        let rest = b"\x1b]0;still opaque";
        rewriter.rewrite(rest, &mut out);
        input.extend_from_slice(rest);
        for byte in terminator {
            rewriter.rewrite(&[*byte], &mut out);
        }
        input.extend_from_slice(terminator);
        assert_eq!(out, input);
        rewriter.rewrite(b"\x1b]2;next\x07", &mut out);
        input.extend_from_slice("\x1b]2;herdr · w1:p2 — next\x07".as_bytes());
        assert_eq!(out, input);
    }
}

#[test]
fn rewrites_two_titles_in_one_chunk() {
    let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
    let mut out = Vec::new();
    rewriter.rewrite(
        b"before\x1b]0;one\x07between\x1b]2;two\x1b\\after",
        &mut out,
    );
    assert_eq!(
        out,
        "before\x1b]0;herdr · w1:p2 — one\x07between\x1b]2;herdr · w1:p2 — two\x1b\\after"
            .as_bytes()
    );
}

#[test]
fn sanitizes_the_prefix_before_rewriting() {
    let mut rewriter = TitleRewriter::new(" \x1bherdr\x07\n · w1:p2 ", None, None);
    let mut out = Vec::new();
    rewriter.rewrite(b"\x1b]2;title\x07", &mut out);
    assert_eq!(out, "\x1b]2;herdr · w1:p2 — title\x07".as_bytes());
}

#[test]
fn flushes_unfinished_sequences_unchanged_at_the_end() {
    for input in [
        b"\x1b".as_slice(),
        b"\x1b]",
        b"\x1b]0;",
        b"\x1b]2;unfinished",
    ] {
        let mut rewriter = TitleRewriter::new("herdr · w1:p2", None, None);
        let mut out = Vec::new();
        rewriter.rewrite(input, &mut out);
        rewriter.finish(&mut out);
        assert_eq!(out, input);
    }
}

#[test]
fn role_and_group_tags_follow_the_prefix_in_every_title() {
    for (role, group, tag) in [
        (Some("planner"), Some(12345), " (planner, group: 12345)"),
        (None, Some(12345), " (group: 12345)"),
        (Some("planner"), None, " (planner)"),
        (None, None, ""),
    ] {
        let mut rewriter = TitleRewriter::new("easy-herdr · w1:p1", group, role.map(str::to_owned));
        assert_eq!(rewriter.label(), format!("easy-herdr · w1:p1{tag}"));
        assert_eq!(
            rewriter.status_title(),
            format!("\x1b]2;easy-herdr · w1:p1{tag}\x07").as_bytes()
        );
        let mut out = Vec::new();
        rewriter.rewrite(b"\x1b]0;Fix CI\x1b\\", &mut out);
        assert_eq!(
            out,
            format!("\x1b]0;easy-herdr · w1:p1{tag} — Fix CI\x1b\\").as_bytes()
        );
        rewriter.set_status(Some("⏳"));
        assert_eq!(
            rewriter.status_title(),
            format!("\x1b]2;⏳ easy-herdr · w1:p1{tag} — Fix CI\x07").as_bytes()
        );
    }
}

#[test]
fn set_role_reports_changes_and_retags_the_title() {
    let mut rewriter = TitleRewriter::new("easy-herdr · w1:p1", Some(7), None);
    assert!(!rewriter.set_role(None));
    assert!(rewriter.set_role(Some("planner".into())));
    assert!(!rewriter.set_role(Some("planner".into())));
    assert_eq!(rewriter.label(), "easy-herdr · w1:p1 (planner, group: 7)");
    assert!(rewriter.set_role(Some("reviewer".into())));
    assert_eq!(
        rewriter.status_title(),
        "\x1b]2;easy-herdr · w1:p1 (reviewer, group: 7)\x07".as_bytes()
    );
    assert!(rewriter.set_role(None));
    assert_eq!(rewriter.label(), "easy-herdr · w1:p1 (group: 7)");
}

#[test]
fn sanitizes_control_characters_in_the_role() {
    let rewriter = TitleRewriter::new(
        "easy-herdr · w1:p1",
        Some(7),
        Some(" \x1bplan\x07ner\n ".into()),
    );
    assert_eq!(rewriter.label(), "easy-herdr · w1:p1 (planner, group: 7)");
    let rewriter = TitleRewriter::new("easy-herdr · w1:p1", None, Some("\x1b\x07\t".into()));
    assert_eq!(rewriter.label(), "easy-herdr · w1:p1");
}
