use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use skim::prelude::*;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::AiTool;
use crate::{config, git, sync, which_exists};

// ── state ───────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct FleetState {
    pub agent: String,
    pub base: Option<String>,
    pub tasks: Vec<FleetTask>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FleetTask {
    pub task: String,
    pub branch: String,
    pub path: String,
}

fn state_path(root: &Path) -> PathBuf {
    root.join(".workz").join("fleet.json")
}

fn load_state(root: &Path) -> Result<FleetState> {
    let path = state_path(root);
    if !path.exists() {
        anyhow::bail!("no active fleet found — run 'workz fleet start' first");
    }
    let raw = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str(&raw)?)
}

/// Try to load fleet state, returning None if no fleet is active.
pub fn try_load_state(root: &Path) -> Option<FleetState> {
    let path = state_path(root);
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn save_state(root: &Path, state: &FleetState) -> Result<()> {
    let dir = root.join(".workz");
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_string_pretty(state)?;
    std::fs::write(state_path(root), json)?;
    Ok(())
}

fn clear_state(root: &Path) {
    let _ = std::fs::remove_file(state_path(root));
}

// ── branch naming ───────────────────────────────────────────────────────

fn task_to_branch(task: &str) -> String {
    let safe: String = task
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = safe
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let truncated = if slug.len() > 40 { &slug[..40] } else { &slug };
    format!("fleet/{}", truncated.trim_end_matches('-'))
}

// ── task file parsing ───────────────────────────────────────────────────

pub fn parse_task_file(path: &Path) -> Result<Vec<String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("could not read task file: {}", path.display()))?;
    let tasks: Vec<String> = content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    if tasks.is_empty() {
        anyhow::bail!("task file is empty or has no valid tasks");
    }
    Ok(tasks)
}

// ── fleet start ─────────────────────────────────────────────────────────

pub fn cmd_start(
    tasks: Vec<String>,
    agent: &AiTool,
    base: Option<&str>,
) -> Result<()> {
    if tasks.is_empty() {
        anyhow::bail!("no tasks provided — use --task \"...\" or --from <file>");
    }

    let root = git::repo_root()?;
    let config = config::load_config(&root)?;
    let n = tasks.len();

    println!("starting fleet: {} task{}", n, if n == 1 { "" } else { "s" });

    let mut state = FleetState {
        agent: agent.to_string(),
        base: base.map(|s| s.to_string()),
        tasks: Vec::new(),
    };

    // Create worktrees sequentially (git locks prevent parallelism here)
    let mut created: Vec<(FleetTask, PathBuf)> = Vec::new();
    for (i, task) in tasks.iter().enumerate() {
        let branch = task_to_branch(task);
        let wt_path = git::worktree_path(&root, &branch);

        print!("  [{}/{}] creating {}... ", i + 1, n, branch);

        if wt_path.exists() {
            println!("already exists");
        } else {
            git::worktree_add(&wt_path, &branch, base)?;
            println!("done");
        }

        // Write task description so agents can read it
        std::fs::write(
            wt_path.join(".workz-task.md"),
            format!("# Task\n\n{}\n", task),
        )?;

        let ft = FleetTask {
            task: task.clone(),
            branch: branch.clone(),
            path: wt_path.to_string_lossy().to_string(),
        };
        created.push((ft.clone(), wt_path));
        state.tasks.push(ft);
    }

    // Sync deps into all worktrees
    println!("\nsyncing dependencies...");
    for (i, (ft, wt_path)) in created.iter().enumerate() {
        print!("  [{}/{}] {}... ", i + 1, n, ft.branch);
        let _framework = sync::sync_worktree(&root, wt_path, &config.sync)?;
        println!("done");
    }

    // Save state before launching agents
    save_state(&root, &state)?;

    // Launch agents (spawn — non-blocking, so all start in parallel)
    println!("\nlaunching {}...", agent);
    for (i, (ft, wt_path)) in created.iter().enumerate() {
        print!("  [{}/{}] {}... ", i + 1, n, ft.branch);
        launch_agent(agent, wt_path)?;
        println!("launched");
    }

    println!("\nfleet ready! {} agent{} running in parallel.", n, if n == 1 { "" } else { "s" });
    println!("  workz fleet status   — check progress");
    println!("  workz fleet run \"npm test\"  — run command in all");
    println!("  workz fleet done     — teardown everything");

    Ok(())
}

fn launch_agent(tool: &AiTool, path: &Path) -> Result<()> {
    let path_str = path.to_str().unwrap_or(".");

    let (cmd, args): (&str, Vec<&str>) = match tool {
        AiTool::Claude => ("claude", vec![]),
        AiTool::Cursor => ("cursor", vec![path_str]),
        AiTool::Code => ("code", vec![path_str]),
        AiTool::Windsurf => ("windsurf", vec![path_str]),
        AiTool::Aider => ("aider", vec![]),
        AiTool::Codex => ("codex", vec![]),
        AiTool::Gemini => ("gemini", vec![]),
    };

    if which_exists(cmd) {
        Command::new(cmd)
            .args(&args)
            .current_dir(path)
            .spawn()?;
    } else {
        eprintln!("  warning: '{}' not found in PATH, skipping", cmd);
    }

    Ok(())
}

// ── fleet status ────────────────────────────────────────────────────────

pub fn cmd_status() -> Result<()> {
    let root = git::repo_root()?;
    let state = load_state(&root)?;

    if state.tasks.is_empty() {
        println!("fleet is empty");
        return Ok(());
    }

    // Launch the dashboard TUI when running interactively; plain text for pipes/scripts
    use std::io::IsTerminal;
    if std::io::stdout().is_terminal() {
        return crate::tui::run_dashboard();
    }

    println!("fleet status  (agent: {})", state.agent);
    println!();

    let max_branch = state.tasks.iter().map(|t| t.branch.len()).max().unwrap_or(0);

    for ft in &state.tasks {
        let wt_path = PathBuf::from(&ft.path);
        let exists = wt_path.exists();

        let dirty = if exists {
            if git::is_dirty(&wt_path).unwrap_or(false) { " [modified]" } else { " [clean]" }
        } else {
            " [missing]"
        };

        let last = if exists {
            git::last_commit_relative(&wt_path)
                .map(|t| format!("  last commit: {}", t))
                .unwrap_or_default()
        } else {
            String::new()
        };

        println!(
            "  {:<width$}  {}{}{}",
            ft.branch,
            ft.task,
            dirty,
            last,
            width = max_branch,
        );
    }

    Ok(())
}

// ── fleet run ───────────────────────────────────────────────────────────

pub fn cmd_run(cmd_parts: &[String]) -> Result<()> {
    let root = git::repo_root()?;
    let state = load_state(&root)?;

    if state.tasks.is_empty() {
        println!("fleet is empty");
        return Ok(());
    }

    let shell_cmd = cmd_parts.join(" ");
    println!("running '{}' in {} worktree{}...", shell_cmd, state.tasks.len(),
        if state.tasks.len() == 1 { "" } else { "s" });

    // Run in each worktree using threads, collect results
    let tasks = state.tasks.clone();
    let handles: Vec<_> = tasks
        .into_iter()
        .map(|ft| {
            let cmd = shell_cmd.clone();
            std::thread::spawn(move || {
                let output = Command::new("sh")
                    .args(["-c", &cmd])
                    .current_dir(&ft.path)
                    .output();
                (ft.branch, output)
            })
        })
        .collect();

    for handle in handles {
        let (branch, result) = handle.join().expect("thread panicked");
        match result {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let status = if out.status.success() { "ok" } else { "FAILED" };
                println!("\n  [{branch}] {status}");
                if !stdout.trim().is_empty() {
                    for line in stdout.trim().lines() {
                        println!("    {}", line);
                    }
                }
                if !stderr.trim().is_empty() {
                    for line in stderr.trim().lines() {
                        eprintln!("    {}", line);
                    }
                }
            }
            Err(e) => println!("\n  [{branch}] error: {e}"),
        }
    }

    Ok(())
}

// ── fleet done ──────────────────────────────────────────────────────────

pub fn cmd_done(force: bool) -> Result<()> {
    let root = git::repo_root()?;
    let state = load_state(&root)?;

    if state.tasks.is_empty() {
        println!("fleet is already empty");
        clear_state(&root);
        return Ok(());
    }

    println!("removing {} fleet worktree{}...", state.tasks.len(),
        if state.tasks.len() == 1 { "" } else { "s" });

    for ft in &state.tasks {
        let wt_path = PathBuf::from(&ft.path);
        print!("  {}... ", ft.branch);
        if !wt_path.exists() {
            println!("already gone");
            continue;
        }
        match git::worktree_remove(&wt_path, force) {
            Ok(_) => println!("removed"),
            Err(e) => eprintln!("warning: {}", e),
        }
    }

    clear_state(&root);
    println!("fleet done!");
    Ok(())
}

// ── fleet merge ─────────────────────────────────────────────────────────

pub fn cmd_merge(base: Option<&str>, squash: bool, all: bool) -> Result<()> {
    let root = git::repo_root()?;
    let state = load_state(&root)?;

    if state.tasks.is_empty() {
        println!("fleet is empty");
        return Ok(());
    }

    let base_branch = base
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.base.clone().unwrap_or_else(git::default_branch));

    let root_branch = git::current_branch(&root).unwrap_or_else(|_| base_branch.clone());

    // Build candidate list with ahead count
    struct Candidate {
        ft: FleetTask,
        ahead: u32,
        is_dirty: bool,
    }

    let mut candidates: Vec<Candidate> = Vec::new();
    for ft in &state.tasks {
        let wt_path = PathBuf::from(&ft.path);
        if !wt_path.exists() {
            eprintln!("  warning: worktree '{}' not found, skipping", ft.branch);
            continue;
        }
        let ahead = git::commits_ahead(&root, &base_branch, &ft.branch).unwrap_or(0);
        let is_dirty = git::is_dirty(&wt_path).unwrap_or(false);
        candidates.push(Candidate { ft: ft.clone(), ahead, is_dirty });
    }

    if candidates.is_empty() {
        println!("no fleet worktrees available");
        return Ok(());
    }

    println!("merging into: {}", root_branch);
    println!();

    let selected_branches: Vec<String> = if all {
        candidates.iter().map(|c| c.ft.branch.clone()).collect()
    } else {
        let lines: Vec<String> = candidates.iter().map(|c| {
            let ahead_str = match c.ahead {
                0 => "no commits ahead".to_string(),
                1 => "1 commit ahead".to_string(),
                n => format!("{} commits ahead", n),
            };
            let dirty = if c.is_dirty { "  [modified]" } else { "" };
            format!("{}\t{}{}\t({})", c.ft.branch, c.ft.task, dirty, ahead_str)
        }).collect();

        let input = lines.join("\n");
        let options = SkimOptionsBuilder::default()
            .height(Some("50%"))
            .multi(true)
            .reverse(true)
            .prompt(Some("select branches to merge (TAB=select, ENTER=confirm)> "))
            .build()
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let item_reader = SkimItemReader::default();
        let items = item_reader.of_bufread(Cursor::new(input));
        let output = Skim::run_with(&options, Some(items));

        match output {
            Some(out) if !out.is_abort && !out.selected_items.is_empty() => out
                .selected_items
                .iter()
                .map(|item| {
                    item.output()
                        .split('\t')
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .collect(),
            _ => {
                println!("cancelled");
                return Ok(());
            }
        }
    };

    if selected_branches.is_empty() {
        println!("nothing selected");
        return Ok(());
    }

    let mode = if squash { "squash" } else { "no-ff" };
    println!(
        "merging {} branch{} (--{})...",
        selected_branches.len(),
        if selected_branches.len() == 1 { "" } else { "es" },
        mode
    );
    println!();

    let mut merged = 0usize;
    for branch in &selected_branches {
        print!("  {} ... ", branch);
        match git::merge_branch(&root, branch, squash) {
            Ok(_) => {
                if squash {
                    let task_msg = state
                        .tasks
                        .iter()
                        .find(|t| &t.branch == branch)
                        .map(|t| t.task.clone())
                        .unwrap_or_else(|| branch.clone());
                    match git::commit_with_message(&root, &task_msg) {
                        Ok(_) => {
                            println!("merged (squashed)");
                            merged += 1;
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            if msg.contains("nothing to commit") || msg.contains("nothing added") {
                                println!("nothing to merge (already up to date)");
                            } else {
                                println!("merge ok, commit failed: {}", e);
                            }
                        }
                    }
                } else {
                    println!("merged");
                    merged += 1;
                }
            }
            Err(e) => {
                println!("CONFLICT");
                eprintln!("    {}", e.to_string().lines().next().unwrap_or(""));
                eprintln!("    resolve conflicts, then run: git commit");
                eprintln!("    re-run 'workz fleet merge' for remaining branches");
                break;
            }
        }
    }

    println!();
    println!("{}/{} branch{} merged", merged, selected_branches.len(),
        if selected_branches.len() == 1 { "" } else { "es" });
    Ok(())
}

// ── fleet pr ────────────────────────────────────────────────────────────

pub fn cmd_pr(base: Option<&str>, draft: bool, all: bool) -> Result<()> {
    if !which_exists("gh") {
        anyhow::bail!("'gh' (GitHub CLI) not found — install it with: brew install gh");
    }

    let root = git::repo_root()?;
    let state = load_state(&root)?;

    if state.tasks.is_empty() {
        println!("fleet is empty");
        return Ok(());
    }

    let base_branch = base
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.base.clone().unwrap_or_else(git::default_branch));

    let candidates: Vec<&FleetTask> = state
        .tasks
        .iter()
        .filter(|ft| PathBuf::from(&ft.path).exists())
        .collect();

    if candidates.is_empty() {
        println!("no fleet worktrees available");
        return Ok(());
    }

    let selected: Vec<&FleetTask> = if all {
        candidates
    } else {
        let lines: Vec<String> = candidates
            .iter()
            .map(|ft| format!("{}\t{}", ft.branch, ft.task))
            .collect();

        let input = lines.join("\n");
        let options = SkimOptionsBuilder::default()
            .height(Some("50%"))
            .multi(true)
            .reverse(true)
            .prompt(Some("select branches to open PRs for (TAB=select, ENTER=confirm)> "))
            .build()
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let item_reader = SkimItemReader::default();
        let items = item_reader.of_bufread(Cursor::new(input));
        let output = Skim::run_with(&options, Some(items));

        let selected_branches: Vec<String> = match output {
            Some(out) if !out.is_abort && !out.selected_items.is_empty() => out
                .selected_items
                .iter()
                .map(|item| {
                    item.output()
                        .split('\t')
                        .next()
                        .unwrap_or("")
                        .to_string()
                })
                .collect(),
            _ => {
                println!("cancelled");
                return Ok(());
            }
        };

        candidates
            .into_iter()
            .filter(|ft| selected_branches.contains(&ft.branch))
            .collect()
    };

    if selected.is_empty() {
        println!("nothing selected");
        return Ok(());
    }

    let draft_str = if draft { " (draft)" } else { "" };
    println!(
        "creating {} PR{}{} against {}...",
        selected.len(),
        if selected.len() == 1 { "" } else { "s" },
        draft_str,
        base_branch
    );
    println!();

    for ft in &selected {
        print!("  {} ... pushing... ", ft.branch);

        if let Err(e) = git::push_branch(&root, &ft.branch) {
            println!("push failed: {}", e.to_string().lines().next().unwrap_or(""));
            continue;
        }

        print!("creating PR... ");

        let body = format!(
            "## Task\n\n{}\n\n---\n*Created by [workz](https://github.com/rohansx/workz) fleet*",
            ft.task
        );
        let base_str = base_branch.as_str();
        let title_str = ft.task.as_str();
        let body_str = body.as_str();

        let mut args: Vec<&str> = vec![
            "pr", "create",
            "--base", base_str,
            "--head", &ft.branch,
            "--title", title_str,
            "--body", body_str,
        ];
        if draft {
            args.push("--draft");
        }

        let output = Command::new("gh").args(&args).current_dir(&root).output();

        match output {
            Ok(out) if out.status.success() => {
                let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
                println!("{}", url);
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                println!("failed: {}", stderr.trim());
            }
            Err(e) => println!("error: {}", e),
        }
    }

    Ok(())
}
