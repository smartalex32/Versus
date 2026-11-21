//! End-to-end Git mergetool protocol checks using a test-only child command.

use std::{
    env, fs,
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
use versus::{
    core::git_merge::{GitMergeModel, MergeChoice},
    load_working_file, save_working_file,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn git_mergetool_child() {
    let Some(mode) = env::var_os("VERSUS_GIT_PROTOCOL_CHILD") else {
        return;
    };
    let path = |name| {
        let path = PathBuf::from(env::var_os(name).unwrap_or_else(|| panic!("missing {name}")));
        if path.is_absolute() {
            path
        } else {
            env::current_dir().unwrap().join(path)
        }
    };
    let local = load_working_file(Some(&path("LOCAL"))).unwrap();
    let remote = load_working_file(Some(&path("REMOTE"))).unwrap();
    let mut result = load_working_file(Some(&path("MERGED"))).unwrap();
    let base = (env::var_os("VERSUS_GIT_HAS_BASE").as_deref() == Some("1".as_ref()))
        .then(|| load_working_file(Some(&path("BASE"))).unwrap().text);
    let mut model = GitMergeModel::new(base, remote.text, local.text, result.text.clone()).unwrap();
    let conflict = model
        .regions()
        .iter()
        .find(|region| region.unresolved)
        .unwrap_or_else(|| panic!("Git supplied no unresolved conflict"));
    model
        .choose(conflict.id, model.revision(), MergeChoice::Incoming)
        .unwrap();
    result.text = model.result().to_owned();
    save_working_file(&mut result).unwrap();
    if mode == "failure" {
        std::process::exit(1);
    }
}

struct Repository(PathBuf);

impl Repository {
    fn new(name: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "versus-git-protocol-{name}-{stamp}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        let repo = Self(path);
        repo.git(&["init", "--initial-branch=main"]);
        repo.git(&["config", "user.email", "tests@example.invalid"]);
        repo.git(&["config", "user.name", "Versus tests"]);
        repo
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn git(&self, args: &[&str]) -> std::process::Output {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("Git must be available for protocol tests");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn git_status(&self, args: &[&str]) -> ExitStatus {
        Command::new("git")
            .args(args)
            .current_dir(&self.0)
            .output()
            .expect("Git must be available for protocol tests")
            .status
    }

    fn commit(&self, message: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", message]);
    }

    fn conflict(&self, name: &str, base: Option<&str>) {
        if let Some(base) = base {
            fs::write(self.file(name), base).unwrap();
            self.commit("base");
        } else {
            fs::write(self.file("tracked.txt"), "base\n").unwrap();
            self.commit("base");
        }
        self.git(&["checkout", "-b", "incoming"]);
        fs::write(self.file(name), "incoming\n").unwrap();
        self.commit("incoming");
        self.git(&["checkout", "main"]);
        fs::write(self.file(name), "current\n").unwrap();
        self.commit("current");
        let output = Command::new("git")
            .args(["merge", "incoming"])
            .current_dir(&self.0)
            .output()
            .unwrap();
        assert!(!output.status.success(), "merge unexpectedly succeeded");
    }

    fn configure_test_tool(&self) {
        let executable = env::current_exe().unwrap();
        let executable = if cfg!(windows) {
            executable.to_string_lossy().replace('\\', "/")
        } else {
            executable.to_string_lossy().into_owned()
        };
        let command = format!(
            "if git cat-file -e \":1:$MERGED\" 2>/dev/null; then has_base=1; else has_base=0; fi; LOCAL=\"$LOCAL\" REMOTE=\"$REMOTE\" BASE=\"$BASE\" MERGED=\"$MERGED\" VERSUS_GIT_HAS_BASE=\"$has_base\" VERSUS_GIT_PROTOCOL_CHILD=\"$VERSUS_GIT_PROTOCOL_CHILD\" {} --exact git_mergetool_child --nocapture",
            shell_quote(&executable),
        );
        self.git(&["config", "mergetool.versus.cmd", &command]);
        self.git(&["config", "mergetool.versus.trustExitCode", "true"]);
        self.git(&["config", "mergetool.versus.hideResolved", "false"]);
    }

    fn run_mergetool(&self, mode: &str) -> std::process::Output {
        Command::new("git")
            .args(["mergetool", "--no-prompt", "--tool=versus"])
            .current_dir(&self.0)
            .env("VERSUS_GIT_PROTOCOL_CHILD", mode)
            .output()
            .unwrap()
    }
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

impl Drop for Repository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn trusted_exit_stages_a_saved_result_and_distinguishes_empty_from_absent_base() {
    for (name, base, has_stage_one) in [("empty-base", Some(""), true), ("add-add", None, false)] {
        let repo = Repository::new(name);
        repo.conflict("conflict.txt", base);
        assert_eq!(
            repo.git_status(&["cat-file", "-e", ":1:conflict.txt"])
                .success(),
            has_stage_one,
        );
        repo.configure_test_tool();
        let output = repo.run_mergetool("success");
        assert!(
            output.status.success(),
            "mergetool failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            fs::read_to_string(repo.file("conflict.txt")).unwrap(),
            "incoming\n"
        );
        assert!(repo.git(&["ls-files", "-u"]).stdout.is_empty());
        assert!(
            String::from_utf8(repo.git(&["diff", "--cached", "--name-only"]).stdout)
                .unwrap()
                .contains("conflict.txt")
        );
    }
}

#[test]
fn failed_tool_exit_restores_the_original_conflict_and_leaves_it_unstaged() {
    let repo = Repository::new("restore-failure");
    repo.conflict("conflict.txt", Some("base\n"));
    let original = fs::read_to_string(repo.file("conflict.txt")).unwrap();
    repo.configure_test_tool();
    let output = repo.run_mergetool("failure");

    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(repo.file("conflict.txt")).unwrap(),
        original
    );
    assert!(!repo.git(&["ls-files", "-u"]).stdout.is_empty());
}
