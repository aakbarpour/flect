//! Safe project-local Codex Skill lifecycle.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use miette::{IntoDiagnostic, Result, WrapErr, miette};
use serde::Serialize;

use crate::SkillCommand;

const INSTALL_ROOT: &str = ".agents/skills/flect";
const OWNED_FILES: &[OwnedFile] = &[
    OwnedFile {
        relative_path: "SKILL.md",
        contents: include_str!("../../../skills/flect/SKILL.md"),
    },
    OwnedFile {
        relative_path: "agents/openai.yaml",
        contents: include_str!("../../../skills/flect/agents/openai.yaml"),
    },
    OwnedFile {
        relative_path: "references/verdicts.md",
        contents: include_str!("../../../skills/flect/references/verdicts.md"),
    },
    OwnedFile {
        relative_path: "references/agent-mode.md",
        contents: include_str!("../../../skills/flect/references/agent-mode.md"),
    },
    OwnedFile {
        relative_path: "references/api-mode.md",
        contents: include_str!("../../../skills/flect/references/api-mode.md"),
    },
    OwnedFile {
        relative_path: "references/isolation.md",
        contents: include_str!("../../../skills/flect/references/isolation.md"),
    },
];

struct OwnedFile {
    relative_path: &'static str,
    contents: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum InstallStatus {
    Current,
    Missing,
    Modified,
}

pub fn status_label(repository_root: &Path) -> Result<String> {
    let install_root = repository_root.join(INSTALL_ROOT);
    reject_symlink_components(repository_root, &install_root)?;
    Ok(status(&install_root)?.to_string())
}

impl std::fmt::Display for InstallStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Current => formatter.write_str("current"),
            Self::Missing => formatter.write_str("missing"),
            Self::Modified => formatter.write_str("modified"),
        }
    }
}

#[derive(Serialize)]
struct LifecycleReport {
    action: &'static str,
    status: InstallStatus,
    path: String,
    changed: bool,
    removed_files: Vec<String>,
    preserved_files: Vec<String>,
}

pub fn run(repository_root: &Path, command: &SkillCommand, json_output: bool) -> Result<()> {
    let install_root = repository_root.join(INSTALL_ROOT);
    reject_symlink_components(repository_root, &install_root)?;
    let report = match command {
        SkillCommand::Install => install(&install_root)?,
        SkillCommand::Status => report("status", &install_root, false, Vec::new(), Vec::new())?,
        SkillCommand::Uninstall => uninstall(&install_root)?,
    };
    if json_output {
        serde_json::to_writer_pretty(std::io::stdout().lock(), &report).into_diagnostic()?;
        println!();
    } else {
        println!("Flect Codex Skill\n");
        println!("Status  {}", report.status);
        println!("Path    {}", report.path);
        if !report.removed_files.is_empty() {
            println!("Removed {} owned file(s)", report.removed_files.len());
        }
        if !report.preserved_files.is_empty() {
            println!(
                "Preserved {} modified or unowned file(s)",
                report.preserved_files.len()
            );
        }
    }
    Ok(())
}

fn reject_symlink_components(repository_root: &Path, install_root: &Path) -> Result<()> {
    let relative = install_root
        .strip_prefix(repository_root)
        .into_diagnostic()
        .wrap_err("Codex Skill install path escaped the repository")?;
    let mut current = repository_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(miette!(
                    "refusing to manage Codex Skill content through symlink {}",
                    current.display()
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error).into_diagnostic(),
        }
    }
    Ok(())
}

fn install(install_root: &Path) -> Result<LifecycleReport> {
    match status(install_root)? {
        InstallStatus::Current => {
            return report("install", install_root, false, Vec::new(), Vec::new());
        }
        InstallStatus::Modified => {
            return Err(miette!(
                "refusing to overwrite modified Codex Skill content at {}; run `flect skill status` and preserve or remove those files explicitly",
                install_root.display()
            ));
        }
        InstallStatus::Missing => {}
    }
    for owned in OWNED_FILES {
        let path = install_root.join(owned.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .into_diagnostic()
                .wrap_err_with(|| format!("could not create {}", parent.display()))?;
        }
        fs::write(&path, owned.contents)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not install {}", path.display()))?;
    }
    report("install", install_root, true, Vec::new(), Vec::new())
}

fn uninstall(install_root: &Path) -> Result<LifecycleReport> {
    if !install_root.exists() {
        return report("uninstall", install_root, false, Vec::new(), Vec::new());
    }
    let mut removed = Vec::new();
    let mut preserved = Vec::new();
    for owned in OWNED_FILES {
        let path = install_root.join(owned.relative_path);
        match fs::read(&path) {
            Ok(bytes) if bytes == owned.contents.as_bytes() => {
                fs::remove_file(&path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("could not remove {}", path.display()))?;
                removed.push(owned.relative_path.to_owned());
            }
            Ok(_) => preserved.push(owned.relative_path.to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).into_diagnostic(),
        }
    }
    remove_empty_directory(&install_root.join("agents"))?;
    remove_empty_directory(&install_root.join("references"))?;
    remove_empty_directory(install_root)?;
    if install_root.exists() {
        for path in relative_files(install_root)? {
            if !preserved.contains(&path) {
                preserved.push(path);
            }
        }
    }
    report(
        "uninstall",
        install_root,
        !removed.is_empty(),
        removed,
        preserved,
    )
}

fn report(
    action: &'static str,
    install_root: &Path,
    changed: bool,
    removed_files: Vec<String>,
    preserved_files: Vec<String>,
) -> Result<LifecycleReport> {
    Ok(LifecycleReport {
        action,
        status: status(install_root)?,
        path: install_root.display().to_string(),
        changed,
        removed_files,
        preserved_files,
    })
}

fn status(install_root: &Path) -> Result<InstallStatus> {
    if !install_root.exists() {
        return Ok(InstallStatus::Missing);
    }
    let expected = OWNED_FILES
        .iter()
        .map(|owned| owned.relative_path.to_owned())
        .collect::<BTreeSet<_>>();
    let actual: BTreeSet<String> = relative_files(install_root)?.into_iter().collect();
    if actual != expected {
        return Ok(InstallStatus::Modified);
    }
    for owned in OWNED_FILES {
        let bytes = fs::read(install_root.join(owned.relative_path))
            .into_diagnostic()
            .wrap_err("could not inspect installed Codex Skill")?;
        if bytes != owned.contents.as_bytes() {
            return Ok(InstallStatus::Modified);
        }
    }
    Ok(InstallStatus::Current)
}

fn relative_files(root: &Path) -> Result<Vec<String>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .into_diagnostic()
            .wrap_err_with(|| format!("could not inspect {}", directory.display()))?
        {
            let entry = entry.into_diagnostic()?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(relative_path(root, &path)?);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn relative_path(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .into_diagnostic()
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
}

fn remove_empty_directory(path: &Path) -> Result<()> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error).into_diagnostic(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_detects_modification() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join(INSTALL_ROOT);
        assert!(install(&root).unwrap().changed);
        assert_eq!(status(&root).unwrap(), InstallStatus::Current);
        assert!(!install(&root).unwrap().changed);
        fs::write(root.join("SKILL.md"), "user modification").unwrap();
        assert_eq!(status(&root).unwrap(), InstallStatus::Modified);
        assert!(install(&root).is_err());
    }

    #[test]
    fn uninstall_removes_only_exact_owned_content() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join(INSTALL_ROOT);
        install(&root).unwrap();
        fs::write(root.join("SKILL.md"), "user modification").unwrap();
        fs::write(root.join("notes.md"), "user content").unwrap();
        let result = uninstall(&root).unwrap();
        assert_eq!(result.status, InstallStatus::Modified);
        assert_eq!(
            fs::read_to_string(root.join("SKILL.md")).unwrap(),
            "user modification"
        );
        assert_eq!(
            fs::read_to_string(root.join("notes.md")).unwrap(),
            "user content"
        );
        assert!(!root.join("agents/openai.yaml").exists());
        assert!(!root.join("references/verdicts.md").exists());
    }

    #[test]
    fn uninstalling_current_install_is_idempotent() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join(INSTALL_ROOT);
        install(&root).unwrap();
        assert_eq!(uninstall(&root).unwrap().status, InstallStatus::Missing);
        assert!(!uninstall(&root).unwrap().changed);
    }
}
