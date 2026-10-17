use super::*;
use std::fs;

#[test]
fn install_creates_an_absolute_symlink_and_is_idempotent() {
    let dir = std::env::temp_dir().join(format!("easy-install-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("easy-herdr");
    fs::write(&exe, "executable").unwrap();
    install_executable(&exe, &dir, "20261017000000").unwrap();
    install_executable(&exe, &dir, "20261017000000").unwrap();
    assert_eq!(
        fs::read_link(dir.join("herdr")).unwrap(),
        exe.canonicalize().unwrap()
    );
    assert!(!dir.join("herdr.bak-20261017000000").exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn install_backs_up_files_directories_and_dangling_links_without_overwriting() {
    let root = std::env::temp_dir().join(format!("easy-backup-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let exe = root.join("easy-herdr");
    fs::write(&exe, "executable").unwrap();
    for kind in ["file", "directory", "symlink"] {
        let dir = root.join(kind);
        fs::create_dir(&dir).unwrap();
        let target = dir.join("herdr");
        match kind {
            "file" => fs::write(&target, "original").unwrap(),
            "directory" => fs::create_dir(&target).unwrap(),
            _ => std::os::unix::fs::symlink("missing", &target).unwrap(),
        }
        let backup = dir.join("herdr.bak-20261017000000");
        fs::write(&backup, "existing backup").unwrap();
        assert_eq!(
            install_executable(&exe, &dir, "20261017000000")
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read_to_string(&backup).unwrap(), "existing backup");
        fs::remove_file(&backup).unwrap();
        install_executable(&exe, &dir, "20261017000000").unwrap();
        assert_eq!(target.canonicalize().unwrap(), exe.canonicalize().unwrap());
        match kind {
            "file" => assert_eq!(fs::read_to_string(&backup).unwrap(), "original"),
            "directory" => assert!(backup.is_dir()),
            _ => assert_eq!(fs::read_link(&backup).unwrap(), Path::new("missing")),
        }
    }
    fs::remove_dir_all(root).unwrap();
}
