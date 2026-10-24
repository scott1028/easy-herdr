const MAX_PENDING_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EscapeState {
    Ground,
    Escape,
    Csi,
    String { osc: bool, escaped: bool },
}

pub(super) struct TitleRewriter {
    prefix: String,
    group: Option<u32>,
    role: Option<String>,
    /// ` (role, group: G)`, ` (group: G)`, ` (role)` or empty.
    tag: String,
    pending: Vec<u8>,
    passthrough_osc: bool,
    previous_escape: bool,
    last_title: Vec<u8>,
    status: Option<&'static str>,
    escape: EscapeState,
    utf8_remaining: u8,
}

impl TitleRewriter {
    pub(super) fn at_ground(&self) -> bool {
        self.escape == EscapeState::Ground && self.utf8_remaining == 0
    }

    pub(super) fn set_status(&mut self, icon: Option<&'static str>) -> bool {
        let changed = self.status != icon;
        self.status = icon;
        changed
    }

    pub(super) fn set_role(&mut self, role: Option<String>) -> bool {
        if self.role == role {
            return false;
        }
        self.role = role;
        self.tag = get_tag(self.role.as_deref(), self.group);
        true
    }

    /// The prefix and its tag, without status or program title.
    pub(super) fn label(&self) -> String {
        format!("{}{}", self.prefix, self.tag)
    }

    pub(super) fn status_title(&self) -> Vec<u8> {
        let mut out = b"\x1b]2;".to_vec();
        self.write_payload(&mut out);
        out.push(7);
        out
    }

    fn write_payload(&self, out: &mut Vec<u8>) {
        if let Some(icon) = self.status {
            out.extend_from_slice(icon.as_bytes());
            out.push(b' ');
        }
        out.extend_from_slice(self.prefix.as_bytes());
        out.extend_from_slice(self.tag.as_bytes());
        if !self.last_title.is_empty() {
            out.extend_from_slice(" — ".as_bytes());
            out.extend_from_slice(&self.last_title);
        }
    }

    pub(super) fn new(prefix: &str, group: Option<u32>, role: Option<String>) -> Self {
        Self {
            prefix: crate::config::sanitize_window_title_text(prefix).unwrap_or_default(),
            group,
            tag: get_tag(role.as_deref(), group),
            role,
            pending: Vec::new(),
            passthrough_osc: false,
            previous_escape: false,
            last_title: Vec::new(),
            status: None,
            escape: EscapeState::Ground,
            utf8_remaining: 0,
        }
    }

    pub(super) fn rewrite(&mut self, mut input: &[u8], out: &mut Vec<u8>) {
        self.track_input(input);
        while !input.is_empty() {
            if self.pending.is_empty() && !self.passthrough_osc {
                let plain = input
                    .iter()
                    .position(|byte| *byte == 0x1b)
                    .unwrap_or(input.len());
                out.extend_from_slice(&input[..plain]);
                input = &input[plain..];
                if input.is_empty() {
                    break;
                }
            }
            let byte = input[0];
            input = &input[1..];
            if self.passthrough_osc {
                out.push(byte);
                if byte == 7 || (self.previous_escape && byte == b'\\') {
                    self.passthrough_osc = false;
                }
                self.previous_escape = byte == 0x1b;
                continue;
            }
            if self.pending.len() == 1 && byte != b']' {
                out.push(0x1b);
                self.pending.clear();
                if byte == 0x1b {
                    self.pending.push(byte);
                } else {
                    out.push(byte);
                }
                continue;
            }
            self.pending.push(byte);
            let ended = byte == 7 || self.pending.ends_with(b"\x1b\\");
            if self.pending.len() > MAX_PENDING_BYTES {
                out.append(&mut self.pending);
                // Keep oversized OSC payloads opaque until their terminator.
                self.passthrough_osc = !ended;
                self.previous_escape = byte == 0x1b;
            } else if ended {
                let terminator_len = if byte == 7 { 1 } else { 2 };
                let payload_end = self.pending.len() - terminator_len;
                if self.pending.starts_with(b"\x1b]0;") || self.pending.starts_with(b"\x1b]2;") {
                    self.last_title.clear();
                    self.last_title
                        .extend_from_slice(&self.pending[4..payload_end]);
                    out.extend_from_slice(&self.pending[..4]);
                    self.write_payload(out);
                    out.extend_from_slice(&self.pending[payload_end..]);
                    self.pending.clear();
                } else {
                    out.append(&mut self.pending);
                }
            }
        }
    }

    fn track_input(&mut self, input: &[u8]) {
        for &byte in input {
            self.utf8_remaining = if self.utf8_remaining > 0 && byte & 0xc0 == 0x80 {
                self.utf8_remaining - 1
            } else {
                match byte {
                    0xc2..=0xdf => 1,
                    0xe0..=0xef => 2,
                    0xf0..=0xf4 => 3,
                    _ => 0,
                }
            };
            self.escape = match self.escape {
                EscapeState::String { osc, escaped } => {
                    if (osc && byte == 7) || (escaped && byte == b'\\') {
                        EscapeState::Ground
                    } else {
                        EscapeState::String {
                            osc,
                            escaped: byte == 0x1b,
                        }
                    }
                }
                _ if byte == 0x1b => EscapeState::Escape,
                EscapeState::Escape => match byte {
                    b'[' => EscapeState::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => EscapeState::String {
                        osc: byte == b']',
                        escaped: false,
                    },
                    0x30..=0x7e => EscapeState::Ground,
                    _ => EscapeState::Escape,
                },
                EscapeState::Csi if !(0x40..=0x7e).contains(&byte) => EscapeState::Csi,
                _ => EscapeState::Ground,
            };
        }
    }

    pub(super) fn finish(&mut self, out: &mut Vec<u8>) {
        out.append(&mut self.pending);
        self.passthrough_osc = false;
        self.previous_escape = false;
        self.escape = EscapeState::Ground;
        self.utf8_remaining = 0;
    }
}

/// The role is sanitized like the prefix; a role with no visible text counts as none.
fn get_tag(role: Option<&str>, group: Option<u32>) -> String {
    let role = role.and_then(crate::config::sanitize_window_title_text);
    match (role, group) {
        (Some(role), Some(group)) => format!(" ({role}, group: {group})"),
        (None, Some(group)) => format!(" (group: {group})"),
        (Some(role), None) => format!(" ({role})"),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
#[path = "passthrough_title_tests.rs"]
mod tests;
