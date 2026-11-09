use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

pub fn generate(root: &Path) -> Result<()> {
    let dir = root.join("ui/src/lib/ipc/bindings");
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("cannot clear {}", dir.display()))?;
    }
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    snapline_services::export_bindings(&dir)?;
    println!("bindings: wrote {}", dir.display());
    Ok(())
}

pub fn check(root: &Path) -> Result<()> {
    let expected = root.join("ui/src/lib/ipc/bindings");
    let work = root.join("target/xtask-bindings");
    if work.exists() {
        fs::remove_dir_all(&work).with_context(|| format!("cannot clear {}", work.display()))?;
    }
    fs::create_dir_all(&work).with_context(|| format!("cannot create {}", work.display()))?;
    snapline_services::export_bindings(&work)?;
    let differences = compare(&expected, &work)?;
    if !differences.is_empty() {
        for difference in &differences {
            eprintln!("bindings: {difference}");
        }
        anyhow::bail!("bindings are out of date; run cargo xtask bindings");
    }
    println!("bindings: up to date");
    Ok(())
}

fn compare(expected: &Path, actual: &Path) -> Result<Vec<String>> {
    let mut differences = Vec::new();
    let expected_files = files(expected)?;
    let actual_files = files(actual)?;
    for name in &expected_files {
        if !actual_files.contains(name) {
            differences.push(format!("missing generated file {name}"));
            continue;
        }
        let left = fs::read(expected.join(name))?;
        let right = fs::read(actual.join(name))?;
        if left != right {
            differences.push(format!("{name} is stale"));
        }
    }
    for name in &actual_files {
        if !expected_files.contains(name) {
            differences.push(format!("unexpected committed file {name}"));
        }
    }
    Ok(differences)
}

fn files(dir: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    if !dir.exists() {
        return Ok(names);
    }
    for entry in fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            for name in files(&path)? {
                names.push(format!("{}/{}", entry.file_name().to_string_lossy(), name));
            }
        } else if path.extension().is_some_and(|value| value == "ts") {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    names.sort();
    Ok(names)
}
