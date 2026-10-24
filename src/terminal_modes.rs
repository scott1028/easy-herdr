use std::io::{self, Write};

const DISABLE_HOST_MOUSE_REPORTING_SEQUENCE: &[u8] =
    b"\x1b[?1006l\x1b[?1016l\x1b[?1015l\x1b[?1005l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?9l";

#[cfg(any(windows, test))]
const WINDOWS_MOUSE_REPORTING_ENABLE_SEQUENCE: &[u8] =
    b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h";
#[cfg(any(windows, test))]
const WINDOWS_MOUSE_REPORTING_DISABLE_SEQUENCE: &[u8] =
    b"\x1b[?1016l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

pub(crate) fn clear_host_mouse_reporting<W: Write>(writer: &mut W) -> io::Result<()> {
    writer.write_all(DISABLE_HOST_MOUSE_REPORTING_SEQUENCE)?;
    writer.flush()
}

/// Pops more kitty keyboard entries than an agent pushes, which empties the stack.
#[cfg(unix)]
const POP_KITTY_KEYBOARD_STACK: &[u8] = b"\x1b[<99u";
/// SGR, cursor, bracketed paste, focus events and modifyOtherKeys back to their defaults;
/// each reset is harmless when the mode is already off.
#[cfg(unix)]
const RESET_PASSTHROUGH_MODES_SEQUENCE: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?2004l\x1b[?1004l\x1b[>4;0m";

/// Undoes the modes a passthrough agent may leave on when it is stopped before its
/// own cleanup. Leaves the alternate screen only when the agent entered it.
#[cfg(unix)]
pub(crate) fn reset_passthrough_modes<W: Write>(
    writer: &mut W,
    leave_alternate_screen: bool,
) -> io::Result<()> {
    // Each screen has its own kitty keyboard stack.
    writer.write_all(POP_KITTY_KEYBOARD_STACK)?;
    if leave_alternate_screen {
        writer.write_all(b"\x1b[?1049l")?;
        writer.write_all(POP_KITTY_KEYBOARD_STACK)?;
    }
    writer.write_all(RESET_PASSTHROUGH_MODES_SEQUENCE)?;
    clear_host_mouse_reporting(writer)
}

/// Follows `CSI ? 1049/1047/47 h/l` in relayed output, across chunk boundaries.
#[cfg(unix)]
#[derive(Debug, Default)]
pub(crate) struct AltScreenTracker {
    is_active: bool,
    scan: AltScreenScan,
}

#[cfg(unix)]
#[derive(Debug, Default, Clone, Copy)]
enum AltScreenScan {
    #[default]
    Ground,
    Escape,
    CsiStart,
    /// Inside `CSI ?`: the parameter so far, and whether an earlier one was an alt screen.
    Private {
        param: u32,
        has_alt: bool,
    },
    /// Inside any other CSI, until its final byte.
    OtherCsi,
}

#[cfg(unix)]
impl AltScreenTracker {
    pub(crate) fn get_is_active(&self) -> bool {
        self.is_active
    }

    pub(crate) fn track(&mut self, bytes: &[u8]) {
        let is_alt = |param: u32| matches!(param, 1049 | 1047 | 47);
        for &byte in bytes {
            self.scan = match (self.scan, byte) {
                (_, 0x1b) => AltScreenScan::Escape,
                // CAN and SUB cancel a sequence without applying it.
                (_, 0x18 | 0x1a) => AltScreenScan::Ground,
                // RIS resets the terminal to the primary screen.
                (AltScreenScan::Escape, b'c') => {
                    self.is_active = false;
                    AltScreenScan::Ground
                }
                (AltScreenScan::Escape, b'[') => AltScreenScan::CsiStart,
                (AltScreenScan::CsiStart, b'?') => AltScreenScan::Private {
                    param: 0,
                    has_alt: false,
                },
                (AltScreenScan::Private { param, has_alt }, b'0'..=b'9') => {
                    AltScreenScan::Private {
                        param: param
                            .saturating_mul(10)
                            .saturating_add(u32::from(byte - b'0')),
                        has_alt,
                    }
                }
                (AltScreenScan::Private { param, has_alt }, b';') => AltScreenScan::Private {
                    param: 0,
                    has_alt: has_alt || is_alt(param),
                },
                (AltScreenScan::Private { param, has_alt }, b'h' | b'l') => {
                    if has_alt || is_alt(param) {
                        self.is_active = byte == b'h';
                    }
                    AltScreenScan::Ground
                }
                (
                    AltScreenScan::CsiStart
                    | AltScreenScan::Private { .. }
                    | AltScreenScan::OtherCsi,
                    0x20..=0x3f,
                ) => AltScreenScan::OtherCsi,
                // C0 controls inside a CSI run without ending it.
                (
                    scan @ (AltScreenScan::CsiStart
                    | AltScreenScan::Private { .. }
                    | AltScreenScan::OtherCsi),
                    0x00..=0x1f,
                ) => scan,
                _ => AltScreenScan::Ground,
            };
        }
    }
}

#[cfg(any(windows, test))]
pub(crate) fn set_windows_mouse_reporting<W: Write>(
    writer: &mut W,
    enabled: bool,
    sgr_pixels: bool,
) -> io::Result<()> {
    if enabled {
        // Keep a true-to-false pixel transition in the same buffered reassertion,
        // before the final SGR enable, so refresh never exposes a disabled gap.
        if !sgr_pixels {
            writer.write_all(b"\x1b[?1016l")?;
        }
        writer.write_all(WINDOWS_MOUSE_REPORTING_ENABLE_SEQUENCE)?;
        if sgr_pixels {
            writer.write_all(b"\x1b[?1016h")?;
        }
    } else {
        writer.write_all(WINDOWS_MOUSE_REPORTING_DISABLE_SEQUENCE)?;
    }
    writer.flush()
}

#[cfg(not(windows))]
pub(crate) fn set_host_kitty_keyboard_report_all<W: Write>(
    writer: &mut W,
    report_all_keys: bool,
) -> io::Result<()> {
    let mut flags = crate::input::ime_compatible_keyboard_enhancement_flags();
    if report_all_keys {
        flags |= crossterm::event::KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
        flags = crossterm::event::KeyboardEnhancementFlags::from_bits_retain(
            flags.bits() | 0b0001_0000,
        );
    }
    crossterm::execute!(
        writer,
        crossterm::event::PopKeyboardEnhancementFlags,
        crossterm::event::PushKeyboardEnhancementFlags(flags)
    )
}

#[cfg(windows)]
pub(crate) fn set_host_kitty_keyboard_report_all<W: Write>(
    _writer: &mut W,
    _report_all_keys: bool,
) -> io::Result<()> {
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DirectHostKeyboardState {
    kitty_flags: Option<u16>,
    modify_other_keys_level: u8,
}

#[cfg(not(windows))]
pub(crate) fn set_direct_host_keyboard_protocol<W: Write>(
    writer: &mut W,
    active: &mut DirectHostKeyboardState,
    next_flags: u16,
    next_modify_other_keys_level: u8,
) -> io::Result<()> {
    let next_kitty_flags = (next_flags != 0).then_some(next_flags);
    if active.kitty_flags == next_kitty_flags
        && active.modify_other_keys_level == next_modify_other_keys_level
    {
        return Ok(());
    }

    if active.kitty_flags != next_kitty_flags {
        if active.kitty_flags.is_some() {
            writer.write_all(b"\x1b[<1u")?;
        }
        if next_flags != 0 {
            write!(writer, "\x1b[>{next_flags}u")?;
        }
    }
    if active.modify_other_keys_level != next_modify_other_keys_level {
        write!(writer, "\x1b[>4;{next_modify_other_keys_level}m")?;
    }
    writer.flush()?;
    *active = DirectHostKeyboardState {
        kitty_flags: next_kitty_flags,
        modify_other_keys_level: next_modify_other_keys_level,
    };
    Ok(())
}

#[cfg(windows)]
pub(crate) fn set_direct_host_keyboard_protocol<W: Write>(
    _writer: &mut W,
    active: &mut DirectHostKeyboardState,
    next_flags: u16,
    next_modify_other_keys_level: u8,
) -> io::Result<()> {
    *active = DirectHostKeyboardState {
        kitty_flags: (next_flags != 0).then_some(next_flags),
        modify_other_keys_level: next_modify_other_keys_level,
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn passthrough_mode_reset_turns_agent_modes_off_and_leaves_only_a_tracked_alt_screen() {
        let modes = b"\x1b[0m\x1b[?25h\x1b[?2004l\x1b[?1004l\x1b[>4;0m".as_slice();
        let mut output = Vec::new();
        reset_passthrough_modes(&mut output, false).unwrap();
        assert_eq!(
            output,
            [b"\x1b[<99u", modes, DISABLE_HOST_MOUSE_REPORTING_SEQUENCE].concat()
        );
        let mut output = Vec::new();
        reset_passthrough_modes(&mut output, true).unwrap();
        assert_eq!(
            output,
            [
                b"\x1b[<99u\x1b[?1049l\x1b[<99u",
                modes,
                DISABLE_HOST_MOUSE_REPORTING_SEQUENCE
            ]
            .concat()
        );
    }

    #[cfg(unix)]
    #[test]
    fn alt_screen_tracker_follows_set_and_reset_split_at_every_byte() {
        for (enter, leave) in [
            ("\x1b[?1049h", "\x1b[?1049l"),
            ("\x1b[?1047h", "\x1b[?1047l"),
            ("\x1b[?47h", "\x1b[?47l"),
            ("\x1b[?1000;1049h", "\x1b[?25;1049l"),
        ] {
            for split in 0..=enter.len() {
                let mut tracker = AltScreenTracker::default();
                tracker.track(&enter.as_bytes()[..split]);
                tracker.track(&enter.as_bytes()[split..]);
                assert!(tracker.get_is_active(), "{enter:?} split at {split}");
                tracker.track(&leave.as_bytes()[..split.min(leave.len())]);
                tracker.track(&leave.as_bytes()[split.min(leave.len())..]);
                assert!(!tracker.get_is_active(), "{leave:?} split at {split}");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn alt_screen_tracker_keeps_its_state_through_cancelled_sequences_and_clears_on_ris() {
        let leaves = |tracker: &AltScreenTracker| {
            let mut output = Vec::new();
            reset_passthrough_modes(&mut output, tracker.get_is_active()).unwrap();
            output.windows(8).any(|bytes| bytes == b"\x1b[?1049l")
        };
        for cancel in ["\x18", "\x1a"] {
            let enter = format!("\x1b[?1049{cancel}h");
            let leave = format!("\x1b[?1049{cancel}l");
            for split in 0..=enter.len() {
                let mut tracker = AltScreenTracker::default();
                tracker.track(&enter.as_bytes()[..split]);
                tracker.track(&enter.as_bytes()[split..]);
                assert!(!tracker.get_is_active(), "{enter:?} split at {split}");
                assert!(!leaves(&tracker));

                let mut tracker = AltScreenTracker::default();
                tracker.track(b"\x1b[?1049h");
                tracker.track(&leave.as_bytes()[..split]);
                tracker.track(&leave.as_bytes()[split..]);
                assert!(tracker.get_is_active(), "{leave:?} split at {split}");
                assert!(leaves(&tracker));
            }
        }
        let mut tracker = AltScreenTracker::default();
        tracker.track(b"\x1b[?1049h\x1b");
        tracker.track(b"c");
        assert!(!tracker.get_is_active());
        assert!(!leaves(&tracker));
    }

    #[cfg(unix)]
    #[test]
    fn alt_screen_tracker_ignores_other_modes_strings_and_aborted_sequences() {
        let mut tracker = AltScreenTracker::default();
        tracker.track(
            b"text\x1b[?25l\x1b[?1000h\x1b[1049h\x1b]2;?1049h\x07\x1b[?10490h\x1b[?104\x1b[?2004h",
        );
        assert!(!tracker.get_is_active());
    }

    #[cfg(not(windows))]
    #[test]
    fn host_keyboard_report_all_replaces_the_current_herdr_stack_entry() {
        let mut output = Vec::new();

        set_host_kitty_keyboard_report_all(&mut output, true).unwrap();
        set_host_kitty_keyboard_report_all(&mut output, false).unwrap();

        assert_eq!(output, b"\x1b[<1u\x1b[>31u\x1b[<1u\x1b[>7u");
    }

    #[cfg(not(windows))]
    #[test]
    fn direct_keyboard_protocol_owns_exactly_one_stack_entry_and_modify_other_keys() {
        let mut output = Vec::new();
        let mut active = DirectHostKeyboardState::default();

        set_direct_host_keyboard_protocol(&mut output, &mut active, 3, 0).unwrap();
        set_direct_host_keyboard_protocol(&mut output, &mut active, 15, 2).unwrap();
        set_direct_host_keyboard_protocol(&mut output, &mut active, 0, 0).unwrap();

        assert_eq!(
            output,
            b"\x1b[>3u\x1b[<1u\x1b[>15u\x1b[>4;2m\x1b[<1u\x1b[>4;0m"
        );
        assert_eq!(active, DirectHostKeyboardState::default());
    }

    #[cfg(not(windows))]
    #[test]
    fn direct_modify_other_keys_works_without_kitty_flags() {
        let mut output = Vec::new();
        let mut active = DirectHostKeyboardState::default();

        set_direct_host_keyboard_protocol(&mut output, &mut active, 0, 1).unwrap();
        set_direct_host_keyboard_protocol(&mut output, &mut active, 0, 2).unwrap();
        set_direct_host_keyboard_protocol(&mut output, &mut active, 0, 0).unwrap();

        assert_eq!(output, b"\x1b[>4;1m\x1b[>4;2m\x1b[>4;0m");
        assert_eq!(active, DirectHostKeyboardState::default());
    }

    #[test]
    fn direct_legacy_keyboard_mode_does_not_pop_the_host_stack() {
        let mut output = Vec::new();
        let mut active = DirectHostKeyboardState::default();

        set_direct_host_keyboard_protocol(&mut output, &mut active, 0, 0).unwrap();

        assert!(output.is_empty());
        assert_eq!(active, DirectHostKeyboardState::default());
    }

    #[test]
    fn clears_all_known_host_mouse_modes() {
        let mut output = Vec::new();
        clear_host_mouse_reporting(&mut output).unwrap();
        let sequence = std::str::from_utf8(&output).unwrap();

        for mode in ["9", "1000", "1002", "1003", "1005", "1006", "1015", "1016"] {
            assert!(
                sequence.contains(&format!("\x1b[?{mode}l")),
                "missing mouse mode {mode}"
            );
        }
    }

    #[test]
    fn windows_mouse_reporting_setup_and_teardown_request_required_modes() {
        let mut output = Vec::new();

        set_windows_mouse_reporting(&mut output, true, true).unwrap();
        set_windows_mouse_reporting(&mut output, true, false).unwrap();
        set_windows_mouse_reporting(&mut output, false, false).unwrap();

        assert_eq!(
            output,
            b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?1016h\x1b[?1016l\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[?1016l\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
        );
    }
}
