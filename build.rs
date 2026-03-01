use std::path::{Path, PathBuf};
use std::process::Command;

const COMMIT_ENV_KEY: &str = "AUTOMATON_GIT_COMMIT";

fn main() {
    println!("cargo:rerun-if-env-changed={COMMIT_ENV_KEY}");

    let commit = env_override_commit()
        .or_else(git_describe_commit)
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env={COMMIT_ENV_KEY}={commit}");

    if let Some(git_dir) = resolve_git_dir() {
        emit_git_rerun_triggers(&git_dir);
    } else {
        println!("cargo:rerun-if-changed=.git/HEAD");
    }
}

fn env_override_commit() -> Option<String> {
    std::env::var(COMMIT_ENV_KEY)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn git_describe_commit() -> Option<String> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let output = Command::new("git")
        .arg("describe")
        .arg("--always")
        .arg("--abbrev=12")
        .arg("--dirty")
        .current_dir(manifest_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    let commit = stdout.trim().to_string();
    if commit.is_empty() {
        return None;
    }
    Some(commit)
}

fn resolve_git_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let git_entry = manifest_dir.join(".git");
    if git_entry.is_dir() {
        return Some(git_entry);
    }
    let contents = std::fs::read_to_string(git_entry).ok()?;
    let raw_path = contents.trim().strip_prefix("gitdir:")?.trim();
    let git_dir = PathBuf::from(raw_path);
    if git_dir.is_absolute() {
        Some(git_dir)
    } else {
        Some(manifest_dir.join(git_dir))
    }
}

fn emit_git_rerun_triggers(git_dir: &Path) {
    let head_path = git_dir.join("HEAD");
    println!("cargo:rerun-if-changed={}", head_path.display());

    let Ok(head_contents) = std::fs::read_to_string(&head_path) else {
        return;
    };
    let Some(reference) = head_contents.trim().strip_prefix("ref:") else {
        return;
    };
    let ref_path = git_dir.join(reference.trim());
    println!("cargo:rerun-if-changed={}", ref_path.display());
}
