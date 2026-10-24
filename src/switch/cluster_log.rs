//! The text log a background cluster writes when it times out.

use std::path::Path;
use std::time::Duration;

use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

use super::cluster_watch::{Outcome, Snapshot};

pub(super) struct LogInfo<'a> {
    pub(super) group: u32,
    pub(super) timeout: Duration,
    pub(super) started: OffsetDateTime,
    pub(super) ended: OffsetDateTime,
    pub(super) cwd: &'a Path,
    pub(super) prompt: &'a str,
    pub(super) outcome: &'a Outcome,
}

/// The log of a timed-out cluster: header, agents, timeline and one screen per live member.
pub(super) fn render(info: &LogInfo) -> String {
    let mut text = format!(
        "# easy-herdr cluster {}\nresult:   timeout ({} elapsed)\nstarted:  {}\n\
         ended:    {}\ncwd:      {}\nprompt:   {}\n\n## agents\n",
        info.group,
        super::format_duration(info.timeout),
        format_time(info.started),
        format_time(info.ended),
        info.cwd.display(),
        info.prompt,
    );
    // Members without a snapshot were gone when the cluster timed out.
    let rows = info.outcome.members.iter().map(|pane_id| {
        let snapshot = info
            .outcome
            .snapshots
            .iter()
            .find(|snapshot| &snapshot.pane_id == pane_id);
        let status = match snapshot {
            Some(Snapshot {
                status: Some(status),
                ..
            }) => format!("{status:?}").to_lowercase(),
            Some(_) => "unknown".into(),
            None => "gone".into(),
        };
        let role = snapshot.and_then(|snapshot| snapshot.name.clone());
        let kind = snapshot.and_then(|snapshot| snapshot.kind.clone());
        [
            pane_id.clone(),
            role.unwrap_or("-".into()),
            kind.unwrap_or("-".into()),
            status,
        ]
    });
    let rows = rows.collect::<Vec<_>>();
    let width = |column: usize| rows.iter().map(|row| row[column].len()).max().unwrap_or(0);
    let (pane_width, role_width, kind_width) = (width(0), width(1), width(2));
    for [pane, role, kind, status] in &rows {
        text.push_str(&format!(
            "{pane:<pane_width$}  {role:<role_width$}  {kind:<kind_width$}  {status}\n"
        ));
    }
    text.push_str("\n## timeline\n");
    for entry in &info.outcome.timeline {
        let time = info.started + entry.elapsed;
        let (hour, minute, second) = time.to_hms();
        text.push_str(&format!(
            "{hour:02}:{minute:02}:{second:02} {}\n",
            entry.event
        ));
    }
    for snapshot in &info.outcome.snapshots {
        let role = snapshot.name.as_deref().unwrap_or("-");
        let screen = snapshot
            .screen
            .as_deref()
            .map(|screen| screen.trim_end_matches('\n'));
        text.push_str(&format!(
            "\n## {} {role} (last 200 lines)\n{}\n",
            snapshot.pane_id,
            screen.unwrap_or("(screen unavailable)")
        ));
    }
    text
}

/// Local time when the offset is known, otherwise UTC.
pub(super) fn now() -> OffsetDateTime {
    let utc = OffsetDateTime::now_utc();
    crate::platform::local_datetime()
        .and_then(|local| {
            let seconds = (local - PrimitiveDateTime::new(utc.date(), utc.time())).whole_seconds();
            // Offsets are whole minutes; the two clock reads differ by under a second.
            let minutes = (seconds + 30).div_euclid(60);
            UtcOffset::from_whole_seconds(i32::try_from(minutes * 60).ok()?).ok()
        })
        .map_or(utc, |offset| utc.to_offset(offset))
}

/// `2026-10-24 15:30:00 +0800`
pub(super) fn format_time(time: OffsetDateTime) -> String {
    let offset = time.offset();
    format!(
        "{} {:02}:{:02}:{:02} {}{:02}{:02}",
        time.date(),
        time.hour(),
        time.minute(),
        time.second(),
        if offset.is_negative() { '-' } else { '+' },
        offset.whole_hours().unsigned_abs(),
        offset.minutes_past_hour().unsigned_abs()
    )
}

/// `20261024-153000`, for the log file name.
pub(super) fn format_file_time(time: OffsetDateTime) -> String {
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        time.year(),
        u8::from(time.month()),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    )
}

#[cfg(test)]
#[path = "cluster_log_tests.rs"]
mod tests;
