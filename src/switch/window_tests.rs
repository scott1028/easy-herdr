use super::*;

#[test]
fn window_shell_argv_keeps_the_shell_as_the_root_process() {
    let root = Path::new("/switch");
    for (shell, expected) in [
        (
            Some("/bin/bash"),
            vec!["/bin/bash", "--rcfile", "/switch/window-bashrc", "-i"],
        ),
        (Some("/usr/bin/zsh"), vec!["/usr/bin/zsh", "-l"]),
        (Some("/usr/bin/fish"), vec!["/usr/bin/fish"]),
        (None, vec!["/bin/sh"]),
    ] {
        assert_eq!(shell_argv(shell, root), expected);
    }
}

#[test]
fn window_bashrc_preserves_login_profile_order_and_ends_with_shims() {
    let root =
        std::env::temp_dir().join(format!("herdr-window-bash-startup-{}", std::process::id()));
    install_startup_files(&root).unwrap();
    let bashrc = fs::read_to_string(root.join("window-bashrc")).unwrap();
    let mut previous = 0;
    for profile in [
        "[ -r /etc/profile ] && . /etc/profile",
        "if [ -r \"$HOME/.bash_profile\" ]; then\n    . \"$HOME/.bash_profile\"",
        "elif [ -r \"$HOME/.bash_login\" ]; then\n    . \"$HOME/.bash_login\"",
        "elif [ -r \"$HOME/.profile\" ]; then\n    . \"$HOME/.profile\"",
    ] {
        let position = bashrc.find(profile).unwrap();
        assert!(position >= previous);
        previous = position;
    }
    assert_eq!(
        bashrc.lines().last(),
        Some("PATH=\"$HERDR_SWITCH_SHIM_DIR:$PATH\"")
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn window_zlogin_restores_shims_and_the_original_zdotdir() {
    let root =
        std::env::temp_dir().join(format!("herdr-window-zsh-startup-{}", std::process::id()));
    install_startup_files(&root).unwrap();
    let zlogin = fs::read_to_string(root.join("window-zsh/.zlogin")).unwrap();
    assert!(zlogin.ends_with(
        r#"PATH="$HERDR_SWITCH_SHIM_DIR:$PATH"
if [ -n "$HERDR_SWITCH_ORIG_ZDOTDIR" ]; then
    ZDOTDIR=$HERDR_SWITCH_ORIG_ZDOTDIR
else
    unset ZDOTDIR
fi
unset HERDR_SWITCH_ORIG_ZDOTDIR HERDR_SWITCH_WINDOW_ZDOTDIR
"#
    ));
    fs::remove_dir_all(root).unwrap();
}
