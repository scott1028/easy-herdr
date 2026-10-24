#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::path::{Path, PathBuf};

/// Runs `easy-herdr install` or `easy-herdr uninstall`.
pub(crate) fn run(command: &str, args: &[String]) -> i32 {
    #[cfg(unix)]
    let result = (|| -> std::io::Result<()> {
        let executable = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .map_or_else(std::env::current_exe, Ok)?
            .canonicalize()?;
        let dir = match args {
            // Use the herdr the shell resolves, so herdr and easy-herdr stay one binary.
            [] => std::env::var_os("PATH")
                .zip(std::env::current_dir().ok())
                .and_then(|(path, cwd)| find_herdr_dir_on_path(&path, &cwd))
                .or_else(|| executable.parent().map(Path::to_path_buf))
                .ok_or_else(|| std::io::Error::other("executable has no directory"))?,
            [flag, dir] if flag == "--dir" => PathBuf::from(dir),
            _ => {
                return Err(std::io::Error::other(format!(
                    "usage: easy-herdr {command} [--dir DIR]"
                )))
            }
        };
        let result = if command == "install" {
            let now = time::OffsetDateTime::now_utc();
            let stamp = format!(
                "{:04}{:02}{:02}{:02}{:02}{:02}",
                now.year(),
                u8::from(now.month()),
                now.day(),
                now.hour(),
                now.minute(),
                now.second()
            );
            install_executable(&executable, &dir, &stamp)
        } else {
            uninstall_executable(&executable, &dir)
        };
        result.map_err(|err| add_dir_hint(err, &dir))
    })();
    #[cfg(not(unix))]
    let result = {
        let _ = args;
        Err::<(), _>(std::io::Error::other(format!(
            "{command} is unsupported on Windows"
        )))
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("easy-herdr {command}: {err}");
            1
        }
    }
}

#[cfg(unix)]
fn install_executable(executable: &Path, dir: &Path, stamp: &str) -> std::io::Result<()> {
    let executable = executable.canonicalize()?;
    let target = dir.join("herdr");
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                && target.canonicalize().ok().as_ref() == Some(&executable)
            {
                println!("already installed: {}", target.display());
                return Ok(());
            }
            let backup = dir.join(format!("herdr.bak-{stamp}"));
            match std::fs::symlink_metadata(&backup) {
                Ok(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        format!("backup already exists: {}", backup.display()),
                    ))
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(err),
            }
            std::fs::rename(&target, &backup)?;
            println!("backed up to {}", backup.display());
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    std::os::unix::fs::symlink(&executable, &target)?;
    println!("installed {} -> {}", target.display(), executable.display());
    Ok(())
}

#[cfg(unix)]
fn uninstall_executable(executable: &Path, dir: &Path) -> std::io::Result<()> {
    let executable = executable.canonicalize()?;
    let target = dir.join("herdr");
    let is_installed = std::fs::symlink_metadata(&target)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
        && target.canonicalize().ok().as_ref() == Some(&executable);
    if !is_installed {
        println!(
            "nothing to uninstall: {} does not link to {}",
            target.display(),
            executable.display()
        );
        return Ok(());
    }
    // Fixed-width UTC stamps sort by time, so the greatest name is the newest backup.
    let newest_backup = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| {
            name.strip_prefix("herdr.bak-").is_some_and(|stamp| {
                stamp.len() == 14 && stamp.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
        .max()
        .map(|name| dir.join(name));
    let plain_backup = dir.join("herdr.bak");
    let backup = newest_backup.or_else(|| {
        std::fs::symlink_metadata(&plain_backup)
            .is_ok()
            .then_some(plain_backup)
    });
    std::fs::remove_file(&target)?;
    println!("removed {}", target.display());
    match backup {
        Some(backup) => {
            std::fs::rename(&backup, &target)?;
            println!("restored {} from {}", target.display(), backup.display());
        }
        None => println!("no backup to restore in {}", dir.display()),
    }
    Ok(())
}

/// Points an error about an unwritable `dir` at `--dir`.
#[cfg(unix)]
fn add_dir_hint(err: std::io::Error, dir: &Path) -> std::io::Error {
    if !matches!(
        err.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem
    ) {
        return err;
    }
    std::io::Error::new(
        err.kind(),
        format!(
            "{}: {err} (use --dir DIR to choose another directory)",
            dir.display()
        ),
    )
}

/// Returns the directory of the first `herdr` the shell would run from `path`.
#[cfg(unix)]
fn find_herdr_dir_on_path(path: &OsStr, cwd: &Path) -> Option<PathBuf> {
    // Like the shell, resolve relative entries from cwd; an empty entry means cwd.
    std::env::split_paths(path)
        .map(|dir| {
            if dir.as_os_str().is_empty() {
                cwd.to_path_buf()
            } else {
                cwd.join(dir)
            }
        })
        .find(|dir| {
            let herdr = dir.join("herdr");
            herdr.metadata().is_ok_and(|file| file.is_file())
                && crate::platform::get_can_execute(&herdr)
        })
}

#[cfg(all(test, unix))]
#[path = "install_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "install_tdd.rs"]
mod install_tdd;
