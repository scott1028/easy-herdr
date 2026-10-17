#[cfg(unix)]
use std::path::{Path, PathBuf};

pub(crate) fn run(args: &[String]) -> i32 {
    #[cfg(unix)]
    let result = (|| -> std::io::Result<()> {
        let executable = std::env::var_os("APPIMAGE")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .map_or_else(std::env::current_exe, Ok)?
            .canonicalize()?;
        let dir = match args {
            [] => executable
                .parent()
                .ok_or_else(|| std::io::Error::other("executable has no directory"))?,
            [flag, dir] if flag == "--dir" => Path::new(dir),
            _ => {
                return Err(std::io::Error::other(
                    "usage: easy-herdr install [--dir DIR]",
                ))
            }
        };
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
        install_executable(&executable, dir, &stamp)
    })();
    #[cfg(not(unix))]
    let result = {
        let _ = args;
        Err::<(), _>(std::io::Error::other("install is unsupported on Windows"))
    };
    match result {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("easy-herdr install: {err}");
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

#[cfg(all(test, unix))]
#[path = "install_tests.rs"]
mod tests;
