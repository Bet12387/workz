use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
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
        sync::sync_worktree(&root, wt_path, &config.sync)?;
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
