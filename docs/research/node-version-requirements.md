# Node Version Requirements for Registered Tools

**Retrieval date:** 2026-09-05  
**Source:** npm registry `latest` dist-tag metadata — `engines.node` is authoritative for npm-distributed versions.

## Registered Tools (`src/registry.rs`)

| Tool name | npm package | Bin | Latest version | `engines.node` | npm metadata source |
|-----------|-------------|-----|----------------|----------------|--------------------|
| codex | `@openai/codex` | codex | **0.153.4** | `>=16` | <https://registry.npmjs.org/@openai/codex/latest> |
| claude-code | `@anthropic-ai/claude-code` | claude | **2.1.261** | `>=22.0.0` | <https://registry.npmjs.org/@anthropic-ai/claude-code/latest> |
| pi | `@earendil-works/pi-coding-agent` | pi | **0.85.0** | `>=22.19.0` | <https://registry.npmjs.org/@earendil-works/pi-coding-agent/latest> |

## Per-Tool Minimum Node.js Version

- **`@openai/codex`** v0.153.4 — Node.js **>= 16** (any 16.x or newer)
- **`@anthropic-ai/claude-code`** v2.1.261 — Node.js **>= 22.0.0**
- **`@earendil-works/pi-coding-agent`** v0.85.0 — Node.js **>= 22.19.0**

## Effective Minimum for All Three Tools

Installing all three registered Tools simultaneously (`install` with no arguments → `all()`) requires satisfying the strictest `engines.node` constraint:

> **Node.js >= 22.19.0**

`@earendil-works/pi-coding-agent` v0.85.0 imposes this floor. It is stricter than `@anthropic-ai/claude-code`'s `>=22.0.0`, which in turn is stricter than `@openai/codex`'s `>=16`. Any Node.js runtime at or above 22.19.0 satisfies all three constraints.

| Package | Constraint | Satisfied by Node >= 22.19.0? |
|---------|-----------|------------------------------|
| @openai/codex | >=16 | Yes |
| @anthropic-ai/claude-code | >=22.0.0 | Yes |
| @earendil-works/pi-coding-agent | >=22.19.0 | Yes (exact floor) |

## Requirements Can Change on Package Updates

The versions and `engines.node` ranges above reflect the npm `latest` dist-tag as of **2026-09-05**. Package publishers can raise (or lower) the Node.js floor on any release. Before relying on these figures, re-verify by querying each package's `latest` metadata:

```
https://registry.npmjs.org/@openai/codex/latest
https://registry.npmjs.org/@anthropic-ai/claude-code/latest
https://registry.npmjs.org/@earendil-works/pi-coding-agent/latest
```

The `engines.node` field in the JSON response is the authoritative constraint for npm-installed versions.

## Methodology

All three packages publish unambiguous `engines.node` fields in their npm registry metadata. Per the research contract, first-party GitHub repositories or package tarball contents were consulted only if metadata were absent or ambiguous — neither condition applied, so no cross-check was necessary.

### First-party repository references (for context, not used for version verification)

- @openai/codex — <https://github.com/openai/codex> (subdirectory `codex-cli`)
- @anthropic-ai/claude-code — <https://github.com/anthropics/claude-code>
- @earendil-works/pi-coding-agent — <https://github.com/earendil-works/pi> (subdirectory `packages/coding-agent`)
