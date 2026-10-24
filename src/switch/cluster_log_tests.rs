use super::super::cluster_watch::{EndReason, TimelineEntry};
use super::*;
use crate::api::schema::AgentStatus;

/// 2026-10-24 07:30:00 UTC at the given offset.
fn at(hours: i8, minutes: i8) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_792_827_000)
        .unwrap()
        .to_offset(UtcOffset::from_hms(hours, minutes, 0).unwrap())
}

#[test]
fn log_times_show_the_local_offset_or_utc() {
    assert_eq!(format_time(at(8, 0)), "2026-10-24 15:30:00 +0800");
    assert_eq!(format_time(at(0, 0)), "2026-10-24 07:30:00 +0000");
    assert_eq!(format_time(at(-5, -30)), "2026-10-24 02:00:00 -0530");
    assert_eq!(format_file_time(at(8, 0)), "20261024-153000");
}

#[test]
fn log_renders_header_aligned_agents_timeline_and_screens() {
    let timeline = [
        (0, "w1:p1 joined"),
        (65, "w1:p12 planner working"),
        (3600, "w1:p3 gone"),
    ]
    .map(|(seconds, event)| TimelineEntry {
        elapsed: Duration::from_secs(seconds),
        event: event.into(),
    });
    let outcome = Outcome {
        reason: EndReason::Timeout,
        closed: 2,
        members: ["w1:p1", "w1:p12", "w1:p3"].map(String::from).to_vec(),
        timeline: timeline.into(),
        snapshots: vec![
            Snapshot {
                pane_id: "w1:p1".into(),
                name: None,
                kind: Some("claude".into()),
                status: Some(AgentStatus::Working),
                screen: Some("line one\nline two\n".into()),
            },
            Snapshot {
                pane_id: "w1:p12".into(),
                name: Some("planner".into()),
                kind: None,
                status: None,
                screen: None,
            },
        ],
    };
    let started = at(8, 0);
    let text = render(&LogInfo {
        group: 12345,
        timeout: Duration::from_secs(7200),
        started,
        ended: started + Duration::from_secs(7200),
        cwd: Path::new("/work/repo"),
        prompt: "fix the bug",
        outcome: &outcome,
    });
    assert_eq!(
        text,
        "\
# easy-herdr cluster 12345
result:   timeout (2h elapsed)
started:  2026-10-24 15:30:00 +0800
ended:    2026-10-24 17:30:00 +0800
cwd:      /work/repo
prompt:   fix the bug

## agents
w1:p1   -        claude  working
w1:p12  planner  -       unknown
w1:p3   -        -       gone

## timeline
15:30:00 w1:p1 joined
15:31:05 w1:p12 planner working
16:30:00 w1:p3 gone

## w1:p1 - (last 200 lines)
line one
line two

## w1:p12 planner (last 200 lines)
(screen unavailable)
"
    );
}
