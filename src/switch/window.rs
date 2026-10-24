//! Shell entry point for a new switch terminal window.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::{
    agent_env, agent_home::create_private_dir, ensure_server, launch_with_retries, shim, with_shims,
};

const BASHRC: &str = r#"[ -r /etc/profile ] && . /etc/profile
if [ -r "$HOME/.bash_profile" ]; then
    . "$HOME/.bash_profile"
elif [ -r "$HOME/.bash_login" ]; then
    . "$HOME/.bash_login"
elif [ -r "$HOME/.profile" ]; then
    . "$HOME/.profile"
fi
PATH="$HERDR_SWITCH_SHIM_DIR:$PATH"
"#;

pub(super) fn launch(
    session: String,
    ready_file: PathBuf,
    label: Option<String>,
    overrides: Vec<(String, String)>,
) -> Result<i32, String> {
    let cwd = std::env::current_dir()
        .and_then(|cwd| cwd.canonicalize())
        .map_err(|err| format!("cannot resolve the working directory: {err}"))?;
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|err| format!("cannot resolve the herdr executable: {err}"))?;
    let root = shim::switch_root()?;
    let shim_dir = shim::install_shims(&root, &exe)?;
    install_startup_files(&root)?;
    let mut env = with_shims(
        agent_env(std::env::vars_os()),
        shim_dir
            .to_str()
            .ok_or("the herdr switch directory is not valid UTF-8")?,
    );
    let shell = env
        .iter()
        .find(|(key, _)| key == "SHELL")
        .map(|(_, shell)| shell.as_str());
    let argv = shell_argv(shell, &root);
    if Path::new(&argv[0])
        .file_name()
        .is_some_and(|name| name == "zsh")
    {
        env.extend([
            (
                "HERDR_SWITCH_ORIG_ZDOTDIR".into(),
                std::env::var("ZDOTDIR").unwrap_or_default(),
            ),
            (
                "ZDOTDIR".into(),
                root.join("window-zsh").display().to_string(),
            ),
        ]);
    }
    env.extend(overrides);
    crate::session::select_session_for_process(&session)?;
    launch_with_retries(ensure_server, || {
        let ready_file = ready_file.clone();
        crate::client::run_passthrough(
            argv.clone(),
            cwd.clone(),
            env.clone(),
            None,
            label.clone(),
            Some(crate::client::PassthroughTitle {
                label: super::switch_session_label(&session),
                group: None,
            }),
            Some(Box::new(move |started| {
                if let Err(err) = write_ready_file(&ready_file, &started.pane_id) {
                    tracing::warn!(path = %ready_file.display(), %err, "cannot write window shell ready file");
                }
            })),
            None,
        )
    })
}

fn shell_argv(shell: Option<&str>, root: &Path) -> Vec<String> {
    let shell = shell.unwrap_or("/bin/sh");
    let mut argv = vec![shell.to_owned()];
    match Path::new(shell).file_name().and_then(|name| name.to_str()) {
        Some("bash") => argv.extend([
            "--rcfile".into(),
            root.join("window-bashrc").display().to_string(),
            "-i".into(),
        ]),
        Some("zsh") => argv.push("-l".into()),
        _ => {}
    }
    argv
}

fn install_startup_files(root: &Path) -> Result<(), String> {
    create_private_dir(root)?;
    let zsh_dir = root.join("window-zsh");
    create_private_dir(&zsh_dir)?;
    let mut files = vec![(root.join("window-bashrc"), BASHRC.to_owned())];
    for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
        let mut wrapper = format!(
            r#"HERDR_SWITCH_WINDOW_ZDOTDIR=$ZDOTDIR
ZDOTDIR=${{HERDR_SWITCH_ORIG_ZDOTDIR:-$HOME}}
[ -r "$ZDOTDIR/{name}" ] && . "$ZDOTDIR/{name}"
ZDOTDIR=$HERDR_SWITCH_WINDOW_ZDOTDIR
"#
        );
        if name == ".zlogin" {
            wrapper.push_str(
                r#"PATH="$HERDR_SWITCH_SHIM_DIR:$PATH"
if [ -n "$HERDR_SWITCH_ORIG_ZDOTDIR" ]; then
    ZDOTDIR=$HERDR_SWITCH_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi
unset HERDR_SWITCH_ORIG_ZDOTDIR HERDR_SWITCH_WINDOW_ZDOTDIR
"#,
            );
        }
        files.push((zsh_dir.join(name), wrapper));
    }
    for (path, content) in files {
        fs::write(&path, content)
            .map_err(|err| format!("cannot write {}: {err}", path.display()))?;
    }
    Ok(())
}

pub(super) fn write_ready_file(path: &Path, pane_id: &str) -> io::Result<()> {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, format!("{pane_id}\n"))?;
    fs::rename(temporary, path)
}

#[cfg(test)]
#[path = "window_tests.rs"]
mod tests;
