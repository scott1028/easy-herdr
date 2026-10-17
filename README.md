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

This creates `herdr` as a symlink beside the current executable. To use another existing, writable directory:

```bash
easy-herdr install --dir ~/.local/bin
```

An existing different `herdr` entry is renamed to `herdr.bak-<timestamp>` (UTC `YYYYmmddHHMMSS`). An existing backup is never overwritten. Re-running is a no-op when the symlink already resolves to the same executable. `install` is unsupported on Windows.

For an AppImage, make the downloaded file executable and run its `install` command, for example:

```bash
./easy-herdr-linux-x86_64.AppImage install
```

AppImages need FUSE, or `APPIMAGE_EXTRACT_AND_RUN=1`. When `APPIMAGE` names an existing file, installation links to that persistent image and defaults to its directory, rather than the temporary mounted executable.

## Quick start

These commands are equivalent:

```bash
easy-herdr codex
easy-herdr switch codex
```

The syntax is:

```text
easy-herdr <agent-cli> [agent-name] [-- agent-args...]
```

`agent-name` names the agent inside herdr; it is not passed to the agent CLI. Put all agent arguments after `--`. For example, replacing `MODEL` with your model name:

```bash
easy-herdr codex reviewer -- --model MODEL
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

Window titles use the launch-directory label and pane ID, followed by the program's title when it supplies one:

```text
{status} {dir} · {pane_id} — {program title}
```

💤 idle · ⏳ working · ✅ done · ✋ blocked. There is no icon for unknown status or no agent. **Done** means finished work and stays visible until the next task begins.

## Differences from herdr

- The binary is named `easy-herdr`; `install` provides the `herdr` symlink for integrations. Switch mode is Linux/macOS only.
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
