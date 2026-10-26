//! Cluster help, with optional terminal color and fixed-width panels.

use std::io::IsTerminal as _;

use crossterm::style::Stylize as _;
use unicode_width::UnicodeWidthStr;

enum Block {
    Plain(&'static str),
    Panel {
        title: &'static str,
        lines: &'static str,
    },
}

const BLOCKS: &[Block] = &[
    Block::Plain(
        r#"Run one or more agents on the same task. They start in the same canonical
directory (symlinks resolved), so they share one switch session: the herdr
session that easy-herdr switch keeps for that directory.

Usage
  easy-herdr cluster [OPTIONS] <AGENT_CLI>... [prompt]
  easy-herdr cluster [OPTIONS] <AGENT_CLI>... -- <prompt words...>

OPTIONS go before the first <AGENT_CLI>."#,
    ),
    Block::Panel {
        title: "What happens",
        lines: r#"easy-herdr cluster claude codex -- "fix the bug"
  │
  ├─► claude   first agent: runs in this terminal and gets the prompt
  └─► codex    every other agent: opens in a new tab, or a new window

The prompt is typed into the first agent once it is detected idle or done;
it is never passed in argv. Readiness is checked every 250 ms for up to 120
seconds. Answer any startup approval dialogs yourself while cluster waits."#,
    },
    Block::Panel {
        title: "Agent names and the prompt",
        lines: r#"Agent names use ASCII letters, digits, ., _, + and -, and cannot start
with -. Agent options are not accepted.
A final argument outside that set is the prompt, unless it starts with -.
Plain names are agents, so put a one-word prompt after --, as in -- review.
Prompts cannot be empty or whitespace-only."#,
    },
    Block::Panel {
        title: "Tabs or windows",
        lines: r#"New tabs open in Konsole, kitty, WezTerm, GNOME Terminal and iTerm2; other
terminals get a new window.
kitty tabs need remote control with a listen socket (listen_on or
KITTY_LISTEN_ON); without it, kitty gets a new window too.
A nonblank HERDR_SWITCH_TERMINAL forces the window command."#,
    },
    Block::Panel {
        title: "Close on its own: --exit-when-done and --background",
        lines: r#"Without these options, nothing closes on its own.

--exit-when-done   Close this cluster's agent panes once every agent has
                   stayed idle for --idle-exit. Prints one line, exits 0.
--timeout <D>      Stop the cluster after D even if agents still work.
                   Prints "timed out" and exits 124. Default: 2h.
                   -1 means the cluster never times out.
--idle-exit <D>    How long every agent must stay idle. Default: 5m.
                   Counting starts after an agent has worked once.
--background       Open no windows: run every agent in the background and
                   return at once. Implies --exit-when-done; needs a
                   prompt. On timeout it writes
                   ./easy-herdr-cluster-<time>-<pid>.log

D is a whole number followed by s, m or h, for example 30s, 90m or 2h.
--timeout and --idle-exit need --exit-when-done or --background."#,
    },
    Block::Panel {
        title: "Run without approval stops: --yolo or --yolo-herdr",
        lines: r#"A dialog is an approval or question prompt that stops an agent until someone
answers it. These options answer dialogs for the first agent only. Pick one;
they cannot be combined.

                agent's own flags   herdr presses keys on
  --yolo        yes                 known dialogs (listed below)
  --yolo-herdr  no                  known dialogs and question forms

--yolo             Start the first agent with flags that skip its approvals.
                   claude*: --permission-mode bypassPermissions; questions
                   continue on their own after --yolo-timeout, rounded up
                   to 60s or 5m, at most 10m (askUserQuestionTimeout).
                   codex*: --ask-for-approval never; sandbox unchanged.
                   Other agents get no extra flags.
--yolo-herdr       Add no agent flags; only herdr presses keys. Question
                   forms, the pick-an-answer prompts of claude and codex,
                   get Enter on the highlighted answer. Not answered:
                   claude edit prompts and codex command approvals, so the
                   first agent waits at those.
--yolo-timeout <D> How long a dialog stays open before herdr presses a key.
                   Default: 60s. 0 presses at once.
                   Needs --yolo or --yolo-herdr.

Known dialogs get their default key: claude Bash and "Do you want to
proceed?" prompts, codex folder trust while "1. Trust and continue" is
selected, and opencode "Permission required". Unknown dialogs are only
logged, and herdr never presses Esc.
claude* means claude, or a name that starts with claude- or claude_;
codex* works the same way.

Three things to keep in mind:
- Agents may run commands without asking: use a sandbox or a separate
  checkout.
- Claude's folder-trust dialog is not answered; use a folder Claude trusts.
- The other agents still stop for approvals; answer those yourself."#,
    },
    Block::Plain(
        r#"Examples
  # Two agents, one prompt; nothing closes on its own
  easy-herdr cluster claude codex -- "fix the bug"

  # Close the panes once every agent is idle; stop after 3h
  easy-herdr cluster --exit-when-done --timeout 3h claude codex -- "fix the bug"

  # Same, but never time out
  easy-herdr cluster --exit-when-done --timeout -1 claude codex -- "fix the bug"

  # No windows; every agent must stay idle 10m before the cluster closes
  easy-herdr cluster --background --timeout 4h --idle-exit 10m \
    claude codex -- "fix the bug"

  # No approval stops, through each agent's own flags
  easy-herdr cluster --yolo claude codex -- "fix the bug"

  # No approval stops, and herdr answers at once with no agent flags
  easy-herdr cluster --yolo-herdr --yolo-timeout 0 claude codex -- "fix the bug"

  # In the background with --yolo; herdr waits 30s before it answers
  easy-herdr cluster --background --timeout 4h --yolo --yolo-timeout 30s \
    claude codex -- "fix the bug""#,
    ),
];

pub(super) fn print_help() {
    let color = get_is_color_enabled(
        std::io::stdout().is_terminal(),
        std::env::var_os("NO_COLOR").as_deref(),
        std::env::var_os("TERM").as_deref(),
    );
    print!("{}", render(color));
}

fn get_is_color_enabled(
    is_terminal: bool,
    no_color: Option<&std::ffi::OsStr>,
    term: Option<&std::ffi::OsStr>,
) -> bool {
    is_terminal
        && no_color.is_none_or(|setting| setting.is_empty())
        && term != Some(std::ffi::OsStr::new("dumb"))
}

fn render(color: bool) -> String {
    let border = |text: String| if color { text.dim().to_string() } else { text };
    let mut blocks = Vec::new();
    for block in BLOCKS {
        let mut output = Vec::new();
        match block {
            Block::Plain(lines) => {
                for line in lines.lines() {
                    output.push(if color && matches!(line, "Usage" | "Examples") {
                        line.bold().to_string()
                    } else if color && line.starts_with("  # ") {
                        line.dim().to_string()
                    } else {
                        line.to_owned()
                    });
                }
            }
            Block::Panel { title, lines } => {
                let fill = "─".repeat(75 - title.width());
                let title = if color {
                    title.bold().to_string()
                } else {
                    (*title).to_owned()
                };
                output.push(format!(
                    "{}{title}{}",
                    border("╭─ ".into()),
                    border(format!(" {fill}╮"))
                ));
                for line in lines.lines() {
                    let padding = " ".repeat(76 - line.width());
                    let text = line.trim_start();
                    let content = if color && text.starts_with("--") {
                        let end = text.find(char::is_whitespace).unwrap_or(text.len());
                        let indent = &line[..line.len() - text.len()];
                        format!("{indent}{}{}", text[..end].cyan().bold(), &text[end..])
                    } else {
                        line.to_owned()
                    };
                    output.push(format!(
                        "{}{content}{padding}{}",
                        border("│ ".into()),
                        border(" │".into())
                    ));
                }
                output.push(border(format!("╰{}╯", "─".repeat(78))));
            }
        }
        blocks.push(output.join("\n"));
    }
    format!("{}\n", blocks.join("\n\n"))
}

#[cfg(test)]
#[path = "cluster_help_tests.rs"]
mod tests;
