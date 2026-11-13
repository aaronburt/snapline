#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod budget;
mod ffmpeg;
mod icons;
mod no_comments;
mod package;
mod tokens;

use std::env;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    let root = no_comments::root()?;
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "help".to_string());
    let rest: Vec<String> = args.collect();
    match command.as_str() {
        "tokens" => tokens_command(&root, &rest),
        "icons" => icons_command(&root, &rest),
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
        "cargo xtask <command>\n\ncommands:\n  tokens       generate the overlay tokens\n  icons        render tray menu icons\n  check        run every static check\n  test         run unit and component tests\n  test-desktop run the ignored desktop tests\n  ci           check, test and budget\n  no-comments  scan for forbidden comments\n  budget       measure resource use\n  fetch-ffmpeg download the pinned FFmpeg build and unpack it\n  package      build the NSIS installer and the portable zip"
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

fn check(root: &Path) -> Result<()> {
    tokens::check(root)?;
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
    Ok(())
}

fn test(root: &Path) -> Result<()> {
    run(root, "cargo", &["nextest", "run", "--workspace"])
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
