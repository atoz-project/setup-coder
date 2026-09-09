# setup-coder

## 开工前必读

- `ARCHITECTURE.md` — 仓库与安装前缀的目录结构约定,动代码前先读
- `CONTEXT.md` — 术语表,输出用词以它为准

## Worktree 约定

需要 git worktree 时,建在 `.agents/worktree/<任务名>/` 下。该路径已 gitignore。

## 提交习惯

改动完成且验证通过后,主动 `git commit` + `git push`(当前分支直推 origin),不等用户提醒。
提交信息沿用仓库惯例:中文 conventional commits(如 `fix(install): …`),一个逻辑改动一个 commit。

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary; label strings equal role names. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
