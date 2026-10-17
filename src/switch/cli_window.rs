//! CLI creates that need a visible switch shell instead of a hidden pane.

use crate::api::{client::ApiClient, schema::*};
use crate::platform::{self, TerminalWindowSpec, TERMINAL_OVERRIDE_ENV};
use serde_json::Value;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn intercept(request: &Request, client: &ApiClient) -> io::Result<Option<Value>> {
    if !matches!(
        request.method,
        Method::PaneSplit(_) | Method::TabCreate(_) | Method::WorkspaceCreate(_)
    ) {
        return Ok(None);
    }
    let Some((session, dir)) = super::active_window_session() else {
        return Ok(None);
    };
    let id = request.id.clone();
    let response = match open_window(request, client, &session, &dir) {
        Ok(result) => serde_json::to_value(SuccessResponse { id, result }),
        Err(error) => serde_json::to_value(ErrorResponse {
            id,
            error: ErrorBody {
                code: "switch_window_failed".into(),
                message: error.to_string(),
            },
        }),
    }
    .map_err(io::Error::other)?;
    Ok(Some(response))
}

fn open_window(
    request: &Request,
    client: &ApiClient,
    session: &str,
    dir: &Path,
) -> io::Result<ResponseResult> {
    let dir = dir.join("switch-windows");
    if let Err(error) = platform::create_remote_private_dir(&dir) {
        if error.kind() != io::ErrorKind::AlreadyExists {
            return Err(error);
        }
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let ready = dir.join(format!("{}-{nanos}.ready", std::process::id()));
    let exe = std::env::current_exe()?.canonicalize()?;
    let cwd = std::env::current_dir()?;
    let spec = window_spec(&request.method, &exe, session, &ready, &cwd)?;
    tracing::debug!(cwd = %spec.cwd.display(), program = ?spec.argv.first(), "opening switch terminal window");
    platform::open_terminal_window(&spec)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.try_exists()? {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, format!("terminal window did not become ready; set {TERMINAL_OVERRIDE_ENV} to override the terminal command")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let pane_id = fs::read_to_string(&ready)?.trim().to_owned();
    fs::remove_file(&ready)?;
    let query = |method| {
        client
            .request(Request {
                id: request.id.clone(),
                method,
            })
            .map(|response| response.result)
            .map_err(io::Error::other)
    };
    let ResponseResult::PaneInfo { pane } = query(Method::PaneGet(PaneTarget { pane_id }))? else {
        return Err(io::Error::other("unexpected pane response"));
    };
    create_response(&request.method, pane, query)
}

fn window_spec(
    method: &Method,
    exe: &Path,
    session: &str,
    ready: &Path,
    fallback_cwd: &Path,
) -> io::Result<TerminalWindowSpec> {
    let (cwd, label, env) = match method {
        Method::PaneSplit(params) => (params.cwd.as_deref(), None, &params.env),
        Method::TabCreate(params) => (params.cwd.as_deref(), params.label.as_deref(), &params.env),
        Method::WorkspaceCreate(params) => {
            (params.cwd.as_deref(), params.label.as_deref(), &params.env)
        }
        _ => return Err(io::Error::other("not a window create request")),
    };
    let mut argv = vec![
        exe.display().to_string(),
        "switch".into(),
        "--window-shell".into(),
        "--session".into(),
        session.into(),
        "--ready-file".into(),
        ready.display().to_string(),
    ];
    if let Some(label) = label {
        argv.extend(["--label".into(), label.into()]);
    }
    for (key, value) in env {
        argv.extend(["--env".into(), format!("{key}={value}")]);
    }
    Ok(TerminalWindowSpec {
        argv,
        cwd: cwd.map_or_else(|| fallback_cwd.to_path_buf(), PathBuf::from),
    })
}

fn create_response(
    method: &Method,
    pane: PaneInfo,
    mut query: impl FnMut(Method) -> io::Result<ResponseResult>,
) -> io::Result<ResponseResult> {
    if matches!(method, Method::PaneSplit(_)) {
        return Ok(ResponseResult::PaneInfo { pane });
    }
    let ResponseResult::TabInfo { tab } = query(Method::TabGet(TabTarget {
        tab_id: pane.tab_id.clone(),
    }))?
    else {
        return Err(io::Error::other("unexpected tab response"));
    };
    if matches!(method, Method::TabCreate(_)) {
        return Ok(ResponseResult::TabCreated {
            tab,
            root_pane: pane,
        });
    }
    let ResponseResult::WorkspaceInfo { workspace } =
        query(Method::WorkspaceGet(WorkspaceTarget {
            workspace_id: pane.workspace_id.clone(),
        }))?
    else {
        return Err(io::Error::other("unexpected workspace response"));
    };
    Ok(ResponseResult::WorkspaceCreated {
        workspace,
        tab,
        root_pane: pane,
    })
}

#[cfg(test)]
#[path = "cli_window_tests.rs"]
mod tests;
