use super::*;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory holding a real agent home and a switch root; removed on drop.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "herdr-switch-home-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("real")).unwrap();
        Self { dir }
    }

    fn real(&self) -> PathBuf {
        self.dir.join("real").canonicalize().unwrap()
    }

    fn root(&self) -> PathBuf {
        self.dir.join("root")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn switch_home_names_are_readable_and_unique_per_real_home() {
    let codex = home_dir_name(Path::new("/home/u/.codex"));
    let personal = home_dir_name(Path::new("/home/u/.codex-personal"));

    assert!(codex.starts_with("codex-"), "{codex}");
    assert!(personal.starts_with("codex-personal-"), "{personal}");
    assert_ne!(
        codex.trim_start_matches("codex-"),
        personal.trim_start_matches("codex-personal-")
    );
    assert_eq!(codex, home_dir_name(Path::new("/home/u/.codex")));
}

#[test]
fn agent_home_is_one_link_to_the_real_home_and_follows_it() {
    let fixture = Fixture::new();
    let real = fixture.real();

    let link = link_agent_home("claude", &real, &fixture.root()).unwrap();

    assert!(link.starts_with(fixture.root().join("claude")), "{link:?}");
    assert_eq!(fs::read_link(&link).unwrap(), real);
    fs::remove_file(&link).unwrap();
    symlink(fixture.dir.join("elsewhere"), &link).unwrap();
    assert_eq!(
        link_agent_home("claude", &real, &fixture.root()).unwrap(),
        link
    );
    assert_eq!(fs::read_link(&link).unwrap(), real);
}

#[test]
fn agent_home_keeps_a_directory_found_in_its_place() {
    let fixture = Fixture::new();
    let real = fixture.real();
    let link = link_agent_home("codex", &real, &fixture.root()).unwrap();
    fs::remove_file(&link).unwrap();
    fs::create_dir(&link).unwrap();
    fs::write(link.join("keep"), "data").unwrap();

    assert_eq!(
        link_agent_home("codex", &real, &fixture.root()).unwrap(),
        link
    );

    assert_eq!(fs::read_to_string(link.join("keep")).unwrap(), "data");
}

#[test]
fn agent_home_links_a_real_home_the_agent_has_not_created_yet() {
    let fixture = Fixture::new();
    let missing = fixture.dir.join(".codex");

    let link = link_agent_home("codex", &missing, &fixture.root()).unwrap();

    assert_eq!(fs::read_link(&link).unwrap(), missing);
    assert!(!missing.exists());
}
