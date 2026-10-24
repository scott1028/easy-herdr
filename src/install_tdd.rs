use super::*;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;

fn create_scratch(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("easy-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_herdr(dir: &Path, mode: u32) {
    fs::create_dir_all(dir).unwrap();
    let herdr = dir.join("herdr");
    fs::write(&herdr, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&herdr, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn find_herdr_dir_on_path_returns_the_first_executable_entry() {
    let root = create_scratch("path-first");
    for name in ["b", "a"] {
        write_herdr(&root.join(name), 0o755);
    }
    let path = std::env::join_paths([root.join("b"), root.join("a")]).unwrap();
    assert_eq!(find_herdr_dir_on_path(&path, &root), Some(root.join("b")));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn find_herdr_dir_on_path_resolves_relative_and_empty_entries_from_cwd() {
    let root = create_scratch("path-relative");
    for name in ["relative", "b"] {
        write_herdr(&root.join(name), 0o755);
    }
    let path = std::env::join_paths([Path::new("relative"), &root.join("b")]).unwrap();
    assert_eq!(
        find_herdr_dir_on_path(&path, &root),
        Some(root.join("relative"))
    );
    let path = std::env::join_paths([Path::new(""), &root.join("b")]).unwrap();
    assert_eq!(
        find_herdr_dir_on_path(&path, &root.join("relative")),
        Some(root.join("relative"))
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn find_herdr_dir_on_path_skips_entries_the_shell_cannot_run() {
    let root = create_scratch("path-skip");
    fs::create_dir_all(root.join("directory/herdr")).unwrap();
    fs::create_dir_all(root.join("dangling")).unwrap();
    std::os::unix::fs::symlink("missing", root.join("dangling/herdr")).unwrap();
    write_herdr(&root.join("plain"), 0o644);
    write_herdr(&root.join("a"), 0o755);
    let skipped = ["directory", "dangling", "plain"].map(|name| root.join(name));
    let path = std::env::join_paths(skipped.iter().chain([&root.join("a")])).unwrap();
    assert_eq!(find_herdr_dir_on_path(&path, &root), Some(root.join("a")));
    let path = std::env::join_paths(&skipped).unwrap();
    assert_eq!(find_herdr_dir_on_path(&path, &root), None);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn find_herdr_dir_on_path_skips_a_herdr_the_current_user_cannot_run() {
    // Root may run any file with an execute bit, as the shell does.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let root = create_scratch("path-owner");
    // Others may execute it, but its owner (the current user) may not.
    write_herdr(&root.join("owner"), 0o601);
    write_herdr(&root.join("a"), 0o755);
    let path = std::env::join_paths([root.join("owner"), root.join("a")]).unwrap();
    assert_eq!(find_herdr_dir_on_path(&path, &root), Some(root.join("a")));
    fs::remove_dir_all(root).unwrap();
}

/// Creates `root/easy-herdr` and links `root/bin/herdr` to it.
fn create_installed(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = create_scratch(name);
    let exe = root.join("easy-herdr");
    fs::write(&exe, "executable").unwrap();
    let dir = root.join("bin");
    fs::create_dir(&dir).unwrap();
    std::os::unix::fs::symlink(&exe, dir.join("herdr")).unwrap();
    (root, exe, dir)
}

#[test]
fn uninstall_restores_the_newest_timestamped_backup() {
    let (root, exe, dir) = create_installed("uninstall-newest");
    for (name, contents) in [
        ("herdr.bak-20261017000000", "older"),
        ("herdr.bak-20261018000000", "newer"),
        ("herdr.bak", "plain"),
        ("herdr.bak-zzz", "decoy"),
    ] {
        fs::write(dir.join(name), contents).unwrap();
    }
    uninstall_executable(&exe, &dir).unwrap();
    assert_eq!(fs::read_to_string(dir.join("herdr")).unwrap(), "newer");
    assert!(!dir.join("herdr").is_symlink());
    assert!(!dir.join("herdr.bak-20261018000000").exists());
    for name in ["herdr.bak-20261017000000", "herdr.bak", "herdr.bak-zzz"] {
        assert!(dir.join(name).exists(), "{name}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn uninstall_falls_back_to_the_plain_backup() {
    let (root, exe, dir) = create_installed("uninstall-plain");
    fs::create_dir(dir.join("herdr.bak")).unwrap();
    uninstall_executable(&exe, &dir).unwrap();
    assert!(dir.join("herdr").is_dir());
    assert!(!dir.join("herdr.bak").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn uninstall_without_a_backup_removes_only_the_link() {
    let (root, exe, dir) = create_installed("uninstall-nobackup");
    uninstall_executable(&exe, &dir).unwrap();
    assert!(fs::symlink_metadata(dir.join("herdr")).is_err());
    assert_eq!(fs::read_to_string(&exe).unwrap(), "executable");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn uninstall_leaves_a_foreign_herdr_untouched() {
    let (root, exe, dir) = create_installed("uninstall-foreign");
    let herdr = dir.join("herdr");
    fs::write(dir.join("herdr.bak"), "backup").unwrap();
    fs::remove_file(&herdr).unwrap();
    fs::write(&herdr, "original").unwrap();
    uninstall_executable(&exe, &dir).unwrap();
    assert_eq!(fs::read_to_string(&herdr).unwrap(), "original");
    fs::remove_file(&herdr).unwrap();
    fs::write(root.join("other"), "other").unwrap();
    std::os::unix::fs::symlink(root.join("other"), &herdr).unwrap();
    uninstall_executable(&exe, &dir).unwrap();
    assert_eq!(fs::read_link(&herdr).unwrap(), root.join("other"));
    assert_eq!(fs::read_to_string(dir.join("herdr.bak")).unwrap(), "backup");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn uninstall_takes_precedence_over_agent_shortcuts() {
    assert!(crate::cli::get_is_command("uninstall"));
}

#[test]
fn add_dir_hint_covers_unwritable_directories_only() {
    let dir = Path::new("/read-only/bin");
    for kind in [
        std::io::ErrorKind::PermissionDenied,
        std::io::ErrorKind::ReadOnlyFilesystem,
    ] {
        let err = add_dir_hint(std::io::Error::from(kind), dir);
        assert_eq!(err.kind(), kind);
        let message = err.to_string();
        assert!(message.starts_with("/read-only/bin: "), "{message}");
        assert!(message.contains("use --dir DIR"), "{message}");
    }
    let err = add_dir_hint(std::io::Error::from(std::io::ErrorKind::NotFound), dir);
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    assert!(!err.to_string().contains("--dir"));
}
