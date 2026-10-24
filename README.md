[English](README.md) · [繁體中文](README.zh-TW.md)

# easy-herdr

<p align="center">
  <img src="assets/herdr.svg" alt="easy-herdr ram mascot icon" width="100" />
</p>

<p align="center">
  <img src="assets/mascot.png" alt="easy-herdr ram mascot" width="960" />
</p>

easy-herdr is a fork of [herdr](https://github.com/herdrdev/herdr). It keeps herdr's features and adds a **switch mode with one native terminal window per agent**, available on Linux and macOS.

For the original features, see the [upstream README pinned to the reference commit](https://github.com/herdrdev/herdr/blob/143bf5fd4781d164260c63fe28e7dc08054ad32f/README.md). A local copy is available at `references/herdr`:

```bash
git submodule update --init references/herdr
```

## Install

Download the asset for your platform from [this fork's Releases](https://github.com/scott1028/herdr/releases):

| Platform | Assets |
| --- | --- |
| Linux x86_64 | `easy-herdr-linux-x86_64`, `easy-herdr-linux-x86_64.AppImage` |
| Linux aarch64 | `easy-herdr-linux-aarch64`, `easy-herdr-linux-aarch64.AppImage` |
| macOS x86_64 | `easy-herdr-macos-x86_64` |
| macOS aarch64 | `easy-herdr-macos-aarch64` |
| Windows x86_64 | `easy-herdr-windows-x86_64.zip` |

On Linux/macOS, rename the bare binary to `easy-herdr`, make it executable with `chmod +x easy-herdr`, and put it on `PATH`. On Windows, extract the ZIP, keep `easy-herdr.exe` beside its ConPTY runtime files, and add that directory to `PATH`.

On Unix, install the `herdr` command that agent skills and hooks call:

```bash
easy-herdr install
```

This replaces the first `herdr` on `PATH` with a symlink to easy-herdr, so `herdr` and `easy-herdr` always run the same binary. When `PATH` has no `herdr`, the symlink goes beside the current executable. To use another existing, writable directory:

```bash
easy-herdr install --dir ~/.local/bin
```

An existing different `herdr` entry is renamed to `herdr.bak-<timestamp>` (UTC `YYYYmmddHHMMSS`). An existing backup is never overwritten. Re-running is a no-op when the symlink already resolves to the same executable. If the directory is not writable, `install` fails without changing anything; rename the old `herdr` yourself, for example with `sudo`, or pass `--dir`. `install` is unsupported on Windows.

To undo it:

```bash
easy-herdr uninstall
```

`uninstall` picks the directory the same way, or takes `--dir DIR`. It removes `herdr` only when it is a symlink to the current easy-herdr, then restores the newest `herdr.bak-<timestamp>`, or `herdr.bak` when there is no timestamped backup. Other backups stay in place. It handles one directory per run. If another directory still has a `herdr` link to easy-herdr, remove it with `easy-herdr uninstall --dir DIR`. When `herdr` is anything else, it changes nothing and exits 0. `uninstall` is unsupported on Windows.

For an AppImage, make the downloaded file executable and run its `install` command, for example:

```bash
./easy-herdr-linux-x86_64.AppImage install
```

AppImages need FUSE, or `APPIMAGE_EXTRACT_AND_RUN=1`. When `APPIMAGE` names an existing file, installation links to that persistent image and, when `PATH` has no `herdr`, defaults to its directory, rather than the temporary mounted executable.

## Quick start

These commands are equivalent:

```bash
easy-herdr codex
easy-herdr switch codex
```

The syntax is:

```text
easy-herdr <agent-cli> [agent-args...]
easy-herdr switch <agent-cli> [agent-name] [-- agent-args...]
```

The shortcut passes every argument to the agent CLI; a single `--` right after `<agent-cli>` is dropped. For example, replacing `MODEL` with your model name:

```bash
easy-herdr codex --model MODEL
```

To name the agent inside herdr, use `switch`. `agent-name` is not passed to the agent CLI; put agent arguments after `--`:

```bash
easy-herdr switch codex reviewer -- --model MODEL
```

The shortcut accepts an executable on `PATH`, or a bash/zsh function or alias loaded by the interactive shell selected by `$SHELL`. Names may contain ASCII letters, digits, `.`, `_`, `+`, and `-`, and cannot start with `-`. Existing easy-herdr commands take precedence. Builtins and keywords alone do not qualify; unavailable names still print `unknown command: <name>` and exit 2.

## Switch mode

Launches from the same canonical directory share one session, so those agents can discover and message one another. Different launch directories use separate sessions.

When agents use the herdr CLI to create panes, tabs, or workspaces in a switch session, each create opens a native terminal window running a switch shell in that session. Closing a switch window ends its pane.

New windows follow the host terminal's environment:

| Platform | Detected terminals | Fallback |
| --- | --- | --- |
| Linux | Ghostty, Konsole, kitty, WezTerm, Alacritty, GNOME Terminal, foot | `x-terminal-emulator -e` |
| macOS | Ghostty, WezTerm, kitty, Alacritty, iTerm2 | Terminal.app |

Override the command used for new windows with `HERDR_SWITCH_TERMINAL`:

```bash
HERDR_SWITCH_TERMINAL="kitty" easy-herdr codex
```

The override is a whitespace-separated command prefix; easy-herdr appends the switch command. Shell quoting within the prefix is not parsed.

New windows load bash login profiles or zsh login startup files. For bash/zsh, the `codex`, `claude`, and `opencode` shims are restored to the front of `PATH` after startup:

- Codex gets `--no-daemon` unless the arguments before `--` already include that flag, a `--remote...` option, or `agents`/`remote-control`.
- OpenCode's interface gets `--standalone` for a private server. Subcommands and explicit `--standalone`, `--no-standalone`, or `--server` choices are left unchanged.
- Claude runs with its original arguments.

This keeps the default Codex/OpenCode tool calls attached to the launch session's environment.

Window titles use the launch-directory label and pane ID, an optional tag, and the program's title when it supplies one:

```text
{status} {dir} · {pane_id} ({role}, herdr-pid: {pid}, cluster: {pid}) — {program title}
```

The tag shows the agent name, `herdr-pid`, and `cluster`. `herdr-pid` is the process ID of the launch directory's herdr server, so it is the same in every window from that directory, including the switch shell windows that agents open when they create panes, tabs, or workspaces. When that server exits while idle, the next launch starts a new one, and windows started after that show its process ID. `cluster` is the cluster's process ID and appears only in windows that `cluster` opens. It is the same number as in the cluster's end line, the timeout log name, and `kill <pid>`. A part that does not apply is left out, and so is the whole tag when none does.

💤 idle · ⏳ working · ✅ done · ✋ blocked. There is no icon for unknown status or no agent. **Done** means finished work and stays visible until the next task begins.

## Cluster

On Unix, `cluster` starts one or more agents from the same canonical directory, so they share one switch session:

```bash
easy-herdr cluster claude codex opencode-local "reply with one word: ready"
easy-herdr cluster claude codex -- hello
```

The first agent runs in your current terminal. Each other agent opens in a new tab when the host terminal supports it, falling back to a new window if the tab command fails:

| Platform | Terminals with tab support | Other terminals |
| --- | --- | --- |
| Linux | Konsole, kitty, WezTerm, GNOME Terminal | New window, including Ghostty |
| macOS | kitty, WezTerm, iTerm2 | New window, including Terminal.app and Ghostty |

kitty needs remote control enabled with a listen socket (`listen_on`, exposed as `KITTY_LISTEN_ON`) to open tabs. Without that socket, cluster opens a new window. A nonblank `HERDR_SWITCH_TERMINAL` uses your configured window command instead of tabs.

The optional prompt goes only to the first agent. easy-herdr polls for a detected idle or done agent every 250 ms, then types the prompt and submits it through herdr's API. It waits up to 120 seconds; answer any startup approval dialogs while it waits. The prompt is never passed in agent arguments.

- Without `--`, the final argument is a prompt if it contains spaces or characters outside the agent-name set above and does not start with `-`. Plain names are always agents; unknown names are errors.
- Use `--` for a one-word prompt, or to join several prompt words. You must provide a prompt after it; empty or whitespace-only prompts are rejected.
- All agents are validated before any terminal opens. Agent options are not accepted; use `easy-herdr cluster --help` for usage.

### Let the cluster close itself

By default, a cluster keeps every agent open after the task until you close the windows yourself. Add `--exit-when-done` and the cluster cleans up after itself. Add `--background` and it also runs without any window, which suits a host you reach only over SSH. Without these options, cluster behaves exactly as before.

Think of it as a meeting-room booking: the room is freed once everyone has been quiet for a while, or when the booking ends at the latest.

```text
start ──► an agent works ──► every agent idle for --idle-exit (5m) ──► close panes, exit 0
start ──► still running when --timeout (2h) ends ──► close panes anyway, exit 124
```

The idle clock starts only after some agent has worked, and any new work resets it. Cluster closes only its own panes, so other agents in the same directory keep running. When nothing else uses the herdr server, it exits by itself about 5 s later.

| Option | Without it | With it |
| --- | --- | --- |
| `--exit-when-done` | Windows and agents stay open after the task; you close them. | Once every agent has stayed idle for `--idle-exit`, cluster closes all of its agent panes. In the foreground it then prints one line and exits 0. |
| `--timeout <D>` | Default `2h`. Takes effect only with `--exit-when-done` or `--background`. | When D has passed and the cluster is still running, it closes its panes anyway. In the foreground it prints `timed out after D` and exits 124; in the background it writes a log in the current directory instead. `-1` removes the deadline: the cluster then ends only when every agent has idled for `--idle-exit` or every agent exits, or when you stop it yourself (`kill <pid>` in the background). |
| `--idle-exit <D>` | Default `5m`, under the same rule. | Sets how long every agent must stay idle before the task counts as done. |
| `--background` | The first agent runs in this terminal; the others open tabs or windows. | No windows: every agent runs in a hidden terminal, and the command prints its process ID and exits 0 at once. You can keep using the terminal or log out. Implies `--exit-when-done` and requires a prompt. |

`D` is a positive whole number followed by `s`, `m` or `h`, such as `30s`, `90m` or `2h`; only `--timeout` also accepts `-1`, which never times out. Put the options before the first agent name, because the last argument after the agents is the prompt.

What you see, and the exit code, depends on how the cluster runs:

| Process | Prints | Exit code |
| --- | --- | --- |
| Foreground cluster | One end line on stdout | 0 when done or when every agent exits, 124 on timeout, 1 if it loses the herdr server |
| Background launcher (the command you type) | The four start lines | 0 right after the start |
| Background supervisor (the detached process) | Nothing, since its stdout goes to `/dev/null`; it writes the timeout log on timeout | 124 on timeout, 143 after `kill`, 1 if a launch step fails, otherwise 0 |

### Which options go together

`--background` already includes `--exit-when-done`, so writing both is allowed, just redundant. Background mode cannot turn auto exit off: nobody is watching it, so without auto exit its agents would run forever.

| Options | Allowed | Result |
| --- | --- | --- |
| none | yes | Same as before |
| `--exit-when-done` | yes | Foreground, timeout 2h, idle 5m |
| `--exit-when-done --timeout 3h --idle-exit 10m` | yes | Foreground, timeout 3h, idle 10m |
| `--background` | yes | Background, timeout 2h, idle 5m |
| `--background --exit-when-done` | yes | Same as the row above |
| `--background --timeout 4h` | yes | Background, timeout 4h; writes a log on timeout |
| `--background --exit-when-done --timeout 4h --idle-exit 10m` | yes | Background, timeout 4h, idle 10m |
| `--exit-when-done --timeout -1` | yes | Foreground, no deadline; ends on idle 5m, when every agent exits, or when you interrupt it |
| `--background --timeout -1` | yes | Background, no deadline; an agent that never idles keeps the cluster running until you `kill <pid>` |
| `--timeout 1h` alone | no, exit 2 | `--timeout requires --exit-when-done or --background` |
| `--idle-exit 10m` alone | no, exit 2 | `--idle-exit requires --exit-when-done or --background` |
| `--exit-when-done --idle-exit -1` | no, exit 2 | `invalid duration: -1 (use 30s, 90m or 2h)` |
| `--background` without a prompt | no, exit 2 | `--background requires a prompt` |
| The same option twice, such as `--timeout 1h --timeout 2h` | no, exit 2 | `--timeout given twice` |

A rejected command opens no window.

### Cluster examples

```bash
# Without the new options: same as before
easy-herdr cluster claude claude claude codex -- "You are the planner. ..."

# Foreground; close everything when done (timeout 2h, idle 5m)
easy-herdr cluster --exit-when-done claude claude claude codex -- "You are the planner. ..."

# Foreground; run at most 3 hours, and count as done after 10 idle minutes
easy-herdr cluster --exit-when-done --timeout 3h --idle-exit 10m claude claude claude codex -- "..."

# Background on an SSH-only host; returns at once, timeout 2h
easy-herdr cluster --background claude claude claude codex -- "..."

# Background with a longer timeout
easy-herdr cluster --background --timeout 4h claude codex -- "..."

# Run as long as the work needs; ends when every agent idles or exits
easy-herdr cluster --exit-when-done --timeout -1 claude codex -- "..."
```

A background start prints how to watch and stop it:

```text
easy-herdr cluster 12345 started in the background
watch: easy-herdr --session sw-easy-herdr-1a2b3c4d5e6f agent list
stop:  kill 12345
log:   ./easy-herdr-cluster-*-12345.log is written only if it times out
```

A foreground cluster prints one line in your terminal when it ends:

```text
easy-herdr cluster 12345: all agents idle for 5m; closed 4 agent panes
easy-herdr cluster 12345: timed out after 2h; closed 4 agent panes
```

`kill <pid>` stops a background cluster cleanly: it closes its panes, removes its working files and writes no log. The timeout log is `./easy-herdr-cluster-<YYYYMMDD-HHMMSS>-<pid>.log`, named after the local start time. It lists the agents, a timeline of their status changes and each agent's last 200 screen lines.

### Cluster window titles

Every window that cluster opens shows the herdr server's process ID as `herdr-pid` and the cluster's process ID as `cluster`. Once an agent has a name, its role shows too:

```text
easy-herdr · w1:p1 (planner, herdr-pid: 39001, cluster: 12345)
```

The role is the herdr agent name. Cluster windows get it from `easy-herdr agent rename <pane> <name>`, for example when the planner renames itself, and the title follows a rename within about 2 s. `easy-herdr switch <cli> <name>` names a plain switch window instead, which shows `(name, herdr-pid: <pid>)` without `cluster`.

### Before you rely on auto exit

When auto exit can guess wrong:

- "Every agent idle" can also mean a planner waiting for your answer. `--idle-exit` is the safety margin.
- An agent CLI that Herdr never detects keeps the cluster running until `--timeout`.

Background mode limits:

- Nobody can answer startup or approval dialogs. A stuck agent shows as blocked, and the cluster ends at `--timeout` with a log.
- If an agent asks for a new pane, tab or workspace, easy-herdr still tries to open a native window, which fails on a host with no GUI.
- It needs a writable working directory for the timeout log; otherwise the command exits 1 at once.

Where to look when something goes wrong:

- Background diagnostics, such as an aborted launch or a failed log write, go to `herdr-cluster.log` in the session's config dir: `~/.config/easy-herdr/sessions/<session>/herdr-cluster.log` (`easy-herdr-dev` for debug builds).
- If systemd-logind uses `KillUserProcesses=yes`, logging out ends every process in your SSH session's scope, a background cluster included. `loginctl enable-linger` alone does not prevent that: it keeps your user manager running, but the cluster must also run outside the session scope. For example, with linger enabled, start it in a user-manager scope: `systemd-run --user --scope easy-herdr cluster --background …`. See [`KillUserProcesses=`](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html#KillUserProcesses=).

## Differences from herdr

- The binary is named `easy-herdr`; `install` provides the `herdr` symlink for integrations, and `uninstall` restores the previous `herdr`. Switch mode is Linux/macOS only.
- On Linux/macOS, config and session data default to `~/.config/easy-herdr`, with state under `~/.local/state/easy-herdr`. Debug builds use `easy-herdr-dev`. `XDG_CONFIG_HOME` and `XDG_STATE_HOME` override the base directories; Windows uses `APPDATA` and `LOCALAPPDATA`.
- If the new `config.toml` is missing, easy-herdr copies only that file once from the legacy directory at the same base path (`herdr`, or `herdr-dev` for debug builds). The usual source is `~/.config/herdr/config.toml`. Sessions and plugins are not migrated.
- There is no binary self-update or background binary version check. `easy-herdr update` and `easy-herdr channel set` print the [Releases link](https://github.com/scott1028/herdr/releases) and exit 1. `easy-herdr channel show` still prints the configured channel and exits 0. Agent-detection manifest updates still run under their existing policy and configuration.

## Build from source

Use the Rust toolchain in [`rust-toolchain.toml`](rust-toolchain.toml) (currently 1.96.1) and Zig 0.16.0:

```bash
ZIG=/path/to/zig cargo build --release --locked
```

The Linux/macOS binary is `target/release/easy-herdr`.

## Releases (maintainers)

Push a tag named `easy-herdr-v<Cargo version>`, optionally followed by `.<n>` or `-<suffix>`. The version must match `Cargo.toml`. Alternatively, dispatch the **easy-herdr release** workflow:

```bash
gh workflow run easy-herdr-release.yml --ref easy-herdr -f tag=easy-herdr-v0.9.5
```

## License

Apache-2.0, the same as upstream. See [LICENSE](LICENSE).
