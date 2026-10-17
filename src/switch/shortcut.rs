use std::io::{self, Read as _};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) fn get_is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
}

pub(super) fn get_is_available(name: &str) -> bool {
    if !get_is_plain_name(name) {
        return false;
    }
    if std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            dir.join(name)
                .metadata()
                .is_ok_and(|file| file.is_file() && file.permissions().mode() & 0o111 != 0)
        })
    }) {
        return true;
    }
    let Some(shell) = std::env::var_os("SHELL") else {
        return false;
    };
    let kind = match Path::new(&shell).file_name().and_then(|name| name.to_str()) {
        Some("bash") => "bash",
        Some("zsh") => "zsh",
        _ => return false,
    };
    let query = if kind == "bash" {
        "type -t"
    } else {
        "whence -w"
    };
    let Ok(mut child) = Command::new(&shell)
        .args([
            "-ic",
            &format!("printf '\\n__EASY_HERDR_TYPE__%s\\n' \"$({query} -- \"$1\")\""),
            "easy-herdr",
            name,
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
    else {
        return false;
    };
    let mut capture = || -> io::Result<Vec<u8>> {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing shell stdout"))?;
        let mut pipe = crate::platform::ChildPipe::new_reader(stdout)?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut output = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let read = pipe.read(&mut buffer);
            let exited = child.try_wait()?.is_some();
            match read {
                Ok(0) if exited => return Ok(output),
                Ok(0) => std::thread::sleep(Duration::from_millis(10)),
                Ok(count) => output.extend_from_slice(&buffer[..count]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    if exited {
                        return Ok(output);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => return Err(err),
            }
            if Instant::now() >= deadline {
                return Err(io::ErrorKind::TimedOut.into());
            }
        }
    };
    let output = capture();
    let _ = child.kill();
    let _ = child.wait();
    output.is_ok_and(|output| get_is_supported_kind(kind, &String::from_utf8_lossy(&output)))
}

pub(super) fn get_is_supported_kind(shell: &str, output: &str) -> bool {
    output
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("__EASY_HERDR_TYPE__"))
        .is_some_and(|kind| match shell {
            "bash" => matches!(kind.trim(), "file" | "function" | "alias"),
            "zsh" => kind.rsplit_once(": ").is_some_and(|(_, kind)| {
                matches!(kind.trim(), "command" | "function" | "alias" | "hashed")
            }),
            _ => false,
        })
}
