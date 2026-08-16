# setup-coder

## 开工前必读

- `ARCHITECTURE.md` — 仓库与安装前缀的目录结构约定,动代码前先读
- `CONTEXT.md` — 术语表,输出用词以它为准

## Worktree 约定

需要 git worktree 时,建在当前编程工具同名的隐藏目录下:`.<工具>/worktree/<任务名>/`。例:Claude Code → `.claude/worktree/xxx/`,pi → `.pi/worktree/xxx/`。这些路径已 gitignore。

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary; label strings equal role names. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
