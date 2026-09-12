# setup-coder

## 开工前必读

- `ARCHITECTURE.md` — 仓库与安装前缀的目录结构约定,动代码前先读
- `CONTEXT.md` — 术语表,输出用词以它为准

## Worktree 约定

需要 git worktree 时,建在 `.agents/worktree/<任务名>/` 下。该路径已 gitignore。

## 提交习惯

改动完成且验证通过后,主动 `git commit` + `git push`(当前分支直推 origin),不等用户提醒。
提交信息沿用仓库惯例:中文 conventional commits(如 `fix(install): …`),一个逻辑改动一个 commit。

## 发版

- 版本唯一源:`Cargo.toml` 的 `version`(`--version` 与下载请求的 User-Agent 都取它)。
- 何时发:有用户可见变更(feat/fix)才发;纯 test/refactor/ci 改动不发。0.x 阶段:feat → minor,fix → patch。
- 步骤:
  1. 单 commit `chore: bump vX.Y.Z`(只改 `Cargo.toml` + `Cargo.lock`:改 version 后跑 `cargo check` 同步 lock)
  2. push main,确认 CI 绿
  3. 打 lightweight tag `git tag vX.Y.Z` 并 `git push origin vX.Y.Z`,触发 release.yml
  4. 盯 release 工作流四平台全绿:`gh run list --limit 1`
- 红线:Release 产物平铺布局是 OSS/Gitee 镜像分发契约(install.sh/ps1 按 `releases/latest/download/` 直链取),不得改文件名与目录结构。

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues, via the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Default five-role vocabulary; label strings equal role names. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: root `CONTEXT.md` + `docs/adr/`. See `docs/agents/domain.md`.
