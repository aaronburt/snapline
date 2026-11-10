#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod bindings;
mod budget;
mod ffmpeg;
mod icons;
mod no_comments;
mod package;
mod tokens;

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let root = no_comments::root()?;
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "ui" => ui(&root),
        "tokens" => tokens_command(&root, &rest),
        "icons" => icons_command(&root, &rest),
        "bindings" => bindings_command(&root, &rest),
        "check" => check(&root),
        "test" => test(&root),
        "test-desktop" => test_desktop(&root),
        "ci" => ci(&root),
        "no-comments" => no_comments::run(&root),
        "budget" => budget::run(&root, &rest),
        "fetch-ffmpeg" => ffmpeg::run(&root),
        "package" => package::run(&root),
        "help" | "--help" | "-h" => {
            help();
            Ok(())
        }
        other => bail!("unknown xtask command: {other}"),
    }
}

fn help() {
    println!(
        "cargo xtask <command>\n\ncommands:\n  ui           install and build the web frontend\n  bindings     generate TypeScript types from the Rust DTOs\n  tokens       generate CSS and overlay tokens\n  icons        render tray menu icons\n  check        run every static check\n  test         run unit and component tests\n  test-desktop run the ignored desktop tests\n  ci           check, test and budget\n  no-comments  scan for forbidden comments\n  budget       measure resource use\n  fetch-ffmpeg download the pinned FFmpeg build and unpack it\n  package      build the NSIS installer and the portable zip"
    );
}

fn run(root: &Path, program: &str, args: &[&str]) -> Result<()> {
    println!("> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("could not start {program}"))?;
    if !status.success() {
        bail!("{program} failed with {status}");
    }
    Ok(())
}

fn run_in(root: &Path, dir: &str, program: &str, args: &[&str]) -> Result<()> {
    println!("> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(root.join(dir))
        .status()
        .with_context(|| format!("could not start {program}"))?;
    if !status.success() {
        bail!("{program} failed with {status}");
    }
    Ok(())
}

fn run_npm(root: &Path, args: &[&str]) -> Result<()> {
    if cfg!(windows) {
        let mut all = vec!["/C", "npm"];
        all.extend_from_slice(args);
        run_in(root, "ui", "cmd", &all)
    } else {
        run_in(root, "ui", "npm", args)
    }
}

fn run_npx(root: &Path, args: &[&str]) -> Result<()> {
    if cfg!(windows) {
        let mut all = vec!["/C", "npx"];
        all.extend_from_slice(args);
        run_in(root, "ui", "cmd", &all)
    } else {
        run_in(root, "ui", "npx", args)
    }
}

fn ui(root: &Path) -> Result<()> {
    let ui_dir = root.join("ui");
    if !ui_dir.join("package.json").exists() {
        bail!("ui/package.json is missing");
    }
    let modules = ui_dir.join("node_modules");
    let lock = ui_dir.join("package-lock.json");
    let installed = modules.join(".package-lock.json");
    let needs_install =
        !modules.exists() || !installed.exists() || modified(&lock)? > modified(&installed)?;
    if needs_install {
        run_npm(root, &["ci"])?;
    }
    run_npm(root, &["run", "build"])
}

fn modified(path: &Path) -> Result<std::time::SystemTime> {
    Ok(fs::metadata(path)
        .with_context(|| format!("cannot read metadata for {}", path.display()))?
        .modified()?)
}

fn tokens_command(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None => tokens::generate(root),
        Some("--check") if args.len() == 1 => tokens::check(root),
        Some(other) => bail!("unknown tokens option: {other}"),
    }
}

fn icons_command(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None => icons::generate(root),
        Some("--check") if args.len() == 1 => icons::check(root),
        Some(other) => bail!("unknown icons option: {other}"),
    }
}

fn bindings_command(root: &Path, args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        None => bindings::generate(root),
        Some("--check") if args.len() == 1 => bindings::check(root),
        Some(other) => bail!("unknown bindings option: {other}"),
    }
}

fn check(root: &Path) -> Result<()> {
    ui(root)?;
    tokens::check(root)?;
    bindings::check(root)?;
    icons::check(root)?;
    no_comments::run(root)?;
    run(root, "cargo", &["fmt", "--all", "--", "--check"])?;
    run(
        root,
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    run(root, "cargo", &["deny", "check"])?;
    run_npx(root, &["prettier", "--check", "."])?;
    run_npx(root, &["svelte-check", "--fail-on-warnings"])?;
    run_npx(
        root,
        &[
            "license-checker-rseidelsohn",
            "--excludePrivatePackages",
            "--onlyAllow",
            "MIT;MIT-0;ISC;BSD-2-Clause;BSD-3-Clause;Apache-2.0;0BSD;CC0-1.0;Unlicense;MPL-2.0;Zlib;Unicode-3.0;GPL-3.0-or-later;LGPL-3.0-or-later;BlueOak-1.0.0;CC-BY-3.0",
        ],
    )?;
    Ok(())
}

fn test(root: &Path) -> Result<()> {
    run(root, "cargo", &["nextest", "run", "--workspace"])?;
    run_npm(root, &["run", "coverage"])
}

fn test_desktop(root: &Path) -> Result<()> {
    run(
        root,
        "cargo",
        &[
            "nextest",
            "run",
            "--workspace",
            "--run-ignored",
            "only",
            "--test-threads",
            "1",
        ],
    )
}

fn ci(root: &Path) -> Result<()> {
    check(root)?;
    test(root)?;
    if let Err(error) = budget::run(root, &[]) {
        eprintln!("budget: guideline check did not pass: {error:#}");
    }
    Ok(())
}
