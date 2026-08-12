use std::fs;
use std::path::Path;
use std::process::Command;

use flect_core::{FileStatus, GitRepository};

#[test]
fn captures_staged_unstaged_and_untracked_files() {
    let repository = repository();
    fs::write(repository.path().join("tracked.txt"), "changed\n").unwrap();
    fs::write(repository.path().join("staged.txt"), "staged\n").unwrap();
    git(repository.path(), ["add", "staged.txt"]);
    fs::write(repository.path().join("untracked.txt"), "untracked\n").unwrap();

    let git_repository = GitRepository::discover(repository.path()).unwrap();
    let patch = git_repository
        .capture_patch(
            &git_repository.head_revision().unwrap(),
            true,
            true,
            1_000_000,
        )
        .unwrap();

    assert_eq!(patch.files.len(), 3);
    assert!(has(&patch, "tracked.txt", FileStatus::Modified));
    assert!(has(&patch, "staged.txt", FileStatus::Added));
    assert!(has(&patch, "untracked.txt", FileStatus::Untracked));
}

#[test]
fn captures_deletion_and_rename() {
    let repository = repository();
    git(repository.path(), ["mv", "tracked.txt", "renamed.txt"]);
    fs::remove_file(repository.path().join("delete-me.txt")).unwrap();

    let git_repository = GitRepository::discover(repository.path()).unwrap();
    let patch = git_repository
        .capture_patch(
            &git_repository.head_revision().unwrap(),
            false,
            true,
            1_000_000,
        )
        .unwrap();

    let renamed = patch
        .files
        .iter()
        .find(|file| file.status == FileStatus::Renamed)
        .unwrap();
    assert_eq!(renamed.old_path.as_deref(), Some("tracked.txt"));
    assert_eq!(renamed.path, "renamed.txt");
    assert!(has(&patch, "delete-me.txt", FileStatus::Deleted));
}

#[test]
fn discovers_repository_from_nested_directory() {
    let repository = repository();
    let nested = repository.path().join("one").join("two");
    fs::create_dir_all(&nested).unwrap();
    let discovered = GitRepository::discover(&nested).unwrap();
    assert_eq!(
        discovered.root().canonicalize().unwrap(),
        repository.path().canonicalize().unwrap()
    );
}

#[test]
fn respects_gitignore_for_untracked_discovery() {
    let repository = repository();
    fs::write(repository.path().join(".gitignore"), "ignored.txt\n").unwrap();
    git(repository.path(), ["add", ".gitignore"]);
    git(repository.path(), ["commit", "-m", "ignore fixture"]);
    fs::write(repository.path().join("ignored.txt"), "private\n").unwrap();

    let git_repository = GitRepository::discover(repository.path()).unwrap();
    let base = git_repository.head_revision().unwrap();
    let respected = git_repository
        .capture_patch(&base, true, true, 1_000_000)
        .unwrap();
    assert!(
        !respected
            .files
            .iter()
            .any(|file| file.path == "ignored.txt")
    );

    let included = git_repository
        .capture_patch(&base, true, false, 1_000_000)
        .unwrap();
    assert!(included.files.iter().any(|file| file.path == "ignored.txt"));
}

#[test]
fn treats_non_utf8_untracked_content_as_binary() {
    let repository = repository();
    fs::write(repository.path().join("binary.dat"), [0xff, 0xfe, 0xfd]).unwrap();
    let git_repository = GitRepository::discover(repository.path()).unwrap();
    let patch = git_repository
        .capture_patch(
            &git_repository.head_revision().unwrap(),
            true,
            true,
            1_000_000,
        )
        .unwrap();
    let file = patch
        .files
        .iter()
        .find(|file| file.path == "binary.dat")
        .unwrap();
    assert!(file.binary);
    assert_eq!(file.insertions, 0);
}

fn repository() -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    git(directory.path(), ["init", "-b", "main"]);
    git(
        directory.path(),
        ["config", "user.email", "tests@flect.local"],
    );
    git(directory.path(), ["config", "user.name", "Flect Tests"]);
    fs::write(directory.path().join("tracked.txt"), "original\n").unwrap();
    fs::write(directory.path().join("delete-me.txt"), "delete\n").unwrap();
    git(directory.path(), ["add", "."]);
    git(directory.path(), ["commit", "-m", "base"]);
    directory
}

fn git<const N: usize>(directory: &Path, arguments: [&str; N]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn has(patch_set: &flect_core::PatchSet, path: &str, status: FileStatus) -> bool {
    patch_set
        .files
        .iter()
        .any(|file| file.path == path && file.status == status)
}
