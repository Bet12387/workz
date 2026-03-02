# workz roadmap

> Vision: evolve from "worktree manager" → "AI development runtime"
> The layer between Git and AI agents that handles isolation, orchestration, and environment management.

---

## shipped

### v0.2
- [x] Skim fuzzy TUI (`workz switch`)
- [x] Global config (`~/.config/workz/config.toml`)
- [x] Smart project detection (Node/Rust/Python/Go/Java)
- [x] Auto-symlink 22 heavy dirs + auto-install from 8 lockfile types
- [x] Docker/Podman compose lifecycle (`--docker`)
- [x] AI agent launch (`--ai` for Claude/Cursor/VS Code)
- [x] Shell tab completions (zsh/bash/fish)
- [x] `workz sync` for existing worktrees
- [x] Lifecycle hooks (`post_start`, `pre_done`)

### v0.3
- [x] IDE config sync — auto-symlink `.vscode/`, `.idea/`, `.cursor/`, `.claude/`, `.zed/`
- [x] `workz status` — rich per-worktree dashboard (branch, dirty, last commit age)
- [x] `workz clean --merged` — auto-prune worktrees whose branches are merged
- [x] More AI launchers — Aider, Codex CLI, Gemini CLI, Windsurf

### v0.4
- [x] `workz mcp` — stdio MCP server exposing 6 tools for AI agents
- [x] `CLAUDE.md` — Claude Code auto-discovers workz tools
- [x] Published to crates.io, Homebrew, awesome-mcp-servers

---

## upcoming

### v0.5 — fleet mode (3-4 weeks) ← HN launch moment
Turns workz into an agent orchestration platform. The killer differentiator vs all competitors.

- [ ] `workz fleet start --task "..." --task "..." --agent claude` — parallel agent launcher
- [ ] `workz fleet status` — ratatui TUI dashboard (worktree, task, agent, status)
- [ ] `workz fleet merge` — interactive merge of completed worktrees
- [ ] `workz fleet pr` — create PR per worktree
- [ ] `workz fleet done` — teardown everything
- [ ] Task file support — `workz fleet start --from tasks.md`

### v0.6 — web dashboard / workz serve (3-4 weeks) ← workz.dev product
Visual layer on top of the CLI. Turns workz into a Conductor competitor, but browser-based and cross-platform.

- [ ] `workz serve` — spin up local web dashboard at `localhost:7777`
- [ ] Worktree cards — branch, dirty/clean, last commit, disk usage, Docker state
- [ ] Agent status — which worktrees have AI agents running
- [ ] Conflict heatmap — visual diff of files modified across worktrees
- [ ] One-click actions — open in terminal / VS Code / Cursor, sync, done
- [ ] workz.dev — landing page + docs site backed by this UI

### v0.7 — environment isolation (3-4 weeks)
The unsolved problem. Code isolation ≠ environment isolation.

- [ ] `--isolated` flag — auto-assign unique PORT, DB_NAME, REDIS_URL per worktree
- [ ] Port registry — `~/.config/workz/ports.json` tracks allocations, avoids conflicts
- [ ] Docker project naming — auto-set `COMPOSE_PROJECT_NAME` per worktree
- [ ] Resource cleanup — `workz done` releases ports, stops containers, optionally drops DB
- [ ] DB name injection — auto-suffix `DB_NAME` with sanitized branch name in .env

### v0.8 — GitHub integration
- [ ] `workz pr` — push + create PR from current worktree
- [ ] `workz done --pr` — create PR + cleanup in one shot
- [ ] `workz review <PR#>` — fetch PR into worktree, sync deps, open editor
- [ ] `workz start --gh-issue 123` — auto-fetch title, create branch
- [ ] CI status in `workz status`

### v1.0 — advanced
- [ ] Cross-worktree conflict pre-detection (`workz conflicts`)
- [ ] `workz clone` — bare-repo + worktree-first setup in one command
- [ ] Cross-worktree search (ripgrep across uncommitted changes)
- [ ] tmux/zellij integration (`--tmux` flag)
- [ ] Monorepo + sparse worktrees (`--scope packages/auth`)
- [ ] Team registry — SQLite showing who/what agent owns which branch

---

## go-to-market

| Phase | Action | Channel |
|-------|--------|---------|
| ✅ v0.3 | Blog: *"I replaced 50 lines of bash with workz start --ai"* | Dev.to, Reddit |
| ✅ v0.4 | Blog: *"AI agents that manage their own worktrees"* | r/ClaudeAI, r/cursor, HN |
| v0.5 | **HN Launch: "Show HN: workz — run 5 AI agents in parallel"** | Hacker News |
| v0.6 | Launch workz.dev dashboard — Conductor killer | Product Hunt |
| v0.7 | Blog: *"The git worktree tools landscape in 2026"* | Dev.to, Reddit |

## competitive position

workz is the **only** tool with: zero-config + auto-detect + auto-symlink + auto-install + Docker + AI + fuzzy TUI + shell integration.

Star gap vs competitors (claude squad 2k, worktrunk 800) is a **distribution problem**, not a product problem.
