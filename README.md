# wikid

[![CI](https://github.com/ihorkatkov/wikid/actions/workflows/ci.yml/badge.svg)](https://github.com/ihorkatkov/wikid/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ihorkatkov/wikid)](https://github.com/ihorkatkov/wikid/releases)
[![Changelog](https://img.shields.io/badge/changelog-CHANGELOG.md-blue)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**wikid: a single Rust binary that exposes plain-Markdown wiki directories to remote agents over CLI and MCP. Give every coding agent on every machine one shared, plain-Markdown knowledge base.**



Your agents, Claude Code, Codex, anything with a shell, already know `ls`, `cat`, `grep`, and surgical line edits. wikid puts those exact primitives on the wire: point `wikid serve` at a directory of Markdown files (an Obsidian vault works as-is) and every agent everywhere reads and writes the same wiki, live. No clone, no pull, no push, no database, no git required, no lock-in — the wiki stays plain files the whole time.

It's the natural home for a [Karpathy-style LLM wiki](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f): a knowledge base your agents maintain for themselves, except now it's one wiki shared across all of them instead of a copy per machine.

## Why

Agents accumulate knowledge — architecture notes, decisions, debugging findings, project context. Today that knowledge is stranded per-machine, or synced through git with all the clone/pull/conflict friction that entails, or locked inside a proprietary tool. wikid takes the simplest possible position:

- **The wiki is a directory of Markdown files.** Nothing else. Zero setup, zero migration, zero export problem.
- **The daemon is dumb.** No LLM inside, no state that isn't derivable from the files. Thinking layers on top, through the same public surface.
- **The surface is the filesystem.** Remote agents lose their native file tools; wikid gives them back the same verbs over HTTP.

## Quickstart

```sh
curl -fsSL https://raw.githubusercontent.com/ihorkatkov/wikid/main/install.sh | bash
```

The installer builds from source with `cargo install`, bootstrapping Rust with rustup if `cargo` is not already available; from a checkout, `./install.sh` works too. Prebuilt binaries for macOS and Linux are available for manual download on the [releases page](https://github.com/ihorkatkov/wikid/releases).

After installing once, update the local `wikid` binary explicitly with:

```sh
wikid update
wikid update --check        # report whether a newer release exists
```

`wikid update` updates only the binary on the machine where it runs. To update a daemon, run it on the server host and restart the daemon/process manager if needed.

### 1. Make (or reuse) a wiki

Any directory of Markdown files is already a wiki — point at an existing Obsidian vault and it just works. Or scaffold a blank LLM wiki:

```sh
wikid init ~/notes
wikid status        # no --dir needed once the wiki is registered in config
```

`init` creates `index.md`, `log.md`, `AGENTS.md`, and `raw/`, `concepts/`, `entities/`, `questions/`, `syntheses/`. It never overwrites existing files, so it's safe in a non-empty directory.

### 2. Serve it

On the machine that owns the wiki:

```sh
cd ~/notes
wikid serve
wikid token show admin       # explicit secret-revealing command
```

If no config exists, `serve` creates `~/.config/wikid/config.toml`, registers the current directory, generates an admin token, and serves immediately. One daemon can serve multiple named wikis — see [docs/wikid.example.toml](docs/wikid.example.toml).

### 3. Use it from anywhere

From any VM, any agent — same commands, same output, over HTTP. Save the connection once in `~/.config/wikid/config.toml`:

```toml
[remotes.notes]
server = "http://127.0.0.1:7448"
token = "wkd_..."
wiki = "notes"
```

Bare `wikid` is an orientation dashboard: it lists every configured local/remote target, marks the active/default target, shows that target's live status, and points new agents to the embedded core guide. Select a configured target with `--target` (or set `default_wiki = "notes"` and omit it):

```sh
wikid                              # all targets + active target status
wikid config list                  # detailed token-safe config inventory
wikid --target notes status        # focused status for one target
wikid --target notes grep "auth flow"
```

```sh
wikid grep "auth flow"
wikid cat architecture.md
wikid cat architecture.md#Decision        # read one heading section
wikid cat log.md --lines 1200-1260        # read a large file window
wikid cat decisions.md --hashes            # each line as line:hash: text
wikid edit decisions.md --line 4 --hash 3b39a78cfdcb --new "status: final"
wikid edit decisions.md --line 5 --hash 9a1b2c3d4e5f --new="- status starts with dash"
printf '%s' '[{"line":4,"expected_hash":"3b39a78cfdcb","new_text":"status: final"}]' \
  | wikid edit-batch decisions.md
```

Focused remote status identifies the configured target, daemon wiki, server URL, and client/server wikid versions. It omits the daemon's filesystem root from human output because that path is not actionable on the client; `status --json` retains the wire-compatible `root` field.

Direct `--server`/`--token`/`--wiki` flags and `WIKID_SERVER`/`WIKID_TOKEN`/`WIKID_WIKI` env vars still work. `--target` is the preferred configured-profile selector. For compatibility, `--wiki` without an explicit `--server` remains an alias for `--target` and ignores `WIKID_SERVER`; with direct `--server`, `--wiki` names the daemon wiki. Network exposure is your choice: localhost, tailscale, or public + TLS.

### Local mode

The same binary works directly on a local directory, no daemon involved:

```sh
wikid --dir ~/notes status
export WIKID_DIR=~/notes
wikid grep "auth flow"
```

## Configuration

One TOML file drives everything — see [docs/wikid.example.toml](docs/wikid.example.toml) for the annotated version. Discovery order: `--config` flag → `$WIKID_CONFIG` → `./wikid.toml` → `~/.config/wikid/config.toml`.

You don't have to write it by hand: both `wikid init` and `wikid serve` bootstrap it. Each creates the config file if none exists (at `~/.config/wikid/config.toml`, or wherever `--config`/`$WIKID_CONFIG` points), registers the wiki directory under a name, and generates an admin token. The token value is never printed during bootstrap — reveal it explicitly with `wikid token show admin`. Editing the file manually works just as well:

```toml
# Address the daemon listens on (default 127.0.0.1:7448).
# Binding beyond loopback requires at least one token.
bind = "127.0.0.1:7448"

# Fallback local wiki or remote profile for zero-target commands.
default_wiki = "notes"

# Wiki name → directory. One daemon serves many wikis;
# every remote call is scoped by name.
[wikis]
notes = "/home/you/notes"
projects = "/home/you/wikis/projects"

# Bearer token → actor name. The token string is the secret.
# Omit the table entirely to serve loopback-only without auth.
[tokens]
"wkd_change_me" = "agent-vm-1"

# Client-only named targets. Select one with `wikid --target projects-wiki`.
# `wiki` defaults to the target name; `token` is optional for auth-less daemons.
[remotes.projects-wiki]
server = "https://projects-wiki.example"
token = "wkd_remote_secret"
wiki = "projects"
```

Inspect all configured local targets and remote servers with `wikid config list` (or `wikid config list --json`). Tokens are never included in this output.

Keep configs containing tokens private (`chmod 600 ~/.config/wikid/config.toml`). On Unix, wikid warns on stderr if a discovered token-bearing config is group/world-accessible. A profile with no `token` falls back to ambient `WIKID_TOKEN`; unset it when the target must receive no Authorization header.

Direct remote mode also exposes flag/env pairs:

| Flag | Env var | Meaning |
|---|---|---|
| `--target` | — | Configured local/remote target name |
| `--server` | `WIKID_SERVER` | Remote daemon URL |
| `--token` | `WIKID_TOKEN` | Bearer token |
| `--wiki` | `WIKID_WIKI` | Daemon wiki with `--server`; legacy config-target alias without it |
| `--dir` | `WIKID_DIR` | Local directory (local mode) |
| `--config` | `WIKID_CONFIG` | Config file path |

With none of these set, wikid reads config and picks the local wiki containing the current directory, the only target across `[wikis]` and `[remotes]`, or unified `default_wiki`. `--target <name>` selects that name across both tables; `--wiki <name>` without a server remains a compatibility alias.

## The surface

Wiki operation commands work identically in local and remote mode. Client-side management commands such as `skills`, `config list`, and `update` do not contact a selected wiki; every command accepts `--json`:

| Command | What it does |
|---|---|
| `skills` | Embedded agent usage guides: list, print, or materialize version-matched SKILL.md files |
| `config list` | List configured local wiki targets and remote servers without revealing tokens |
| bare `wikid` | Orientation dashboard: all configured targets plus active target status and agent-guide hint |
| `status` | Focused page counts, recent activity, health summary for one target |
| `ls` / `tree` / `glob` | Find pages by path |
| `cat` | Read a page or `#Heading` / `#^block-id` fragment (large whole-page reads truncated with a size hint; `--full` or `--lines START-END` to override) |
| `grep` | Regex search with ranked results and match context |
| `write` / `edit` / `edit-batch` | Create pages; hash-guarded line edits — a stale hash refuses the whole edit batch, so concurrent writers never silently clobber each other |
| `mv` / `rm` | Rename and delete (`rm` requires `--force` — never an interactive prompt) |
| `links` | Outgoing links and backlinks from the wikilink graph |
| `tags` | Inline/frontmatter tags; nested-tag ancestors are included and marked when only implied |
| `doctor` | Structural health checks: broken wikilinks, orphans, stale and oversized pages |
| `update` | Explicitly update the installed local `wikid` binary from verified GitHub release assets |

Output follows the [AXI principles](https://axi.md/) for agent-facing CLIs: token-efficient, content-first, structured errors on stdout, exit codes 0/1/2, contextual next-step hints, never interactive.

## Giving your agents the wiki

wikid ships agent usage guides inside the binary, so the instructions are always version-matched with the installed CLI:

```sh
wikid skills
wikid skills get core
wikid skills get core --full
```

A minimal `CLAUDE.md` / `AGENTS.md` pointer can stay short:

```markdown
## Shared wiki

Before using wikid, run `wikid skills get core` and follow that guide.
```

To expose the embedded guide as a Claude Code skill, materialize it and symlink the skill directory; the CLI guide name is `core`, while the skill frontmatter name (and conventional symlink name) is `wikid-core`:

```sh
ln -s "$(wikid skills path core)" ~/.claude/skills/wikid-core
wikid skills status
```

Wiring is version-independent: `wikid skills path` prints a path routed through `current`, so updating wikid refreshes the guides behind the same symlink.

`examples/llm-wiki/` is a small public-safe demo vault showing the full pattern — raw-source intake, compiled concept pages, an index, a log, and clean wikilinks:

```sh
wikid --dir examples/llm-wiki status
wikid --dir examples/llm-wiki links index.md
wikid --dir examples/llm-wiki doctor
```

## Design

- **Plain files are the product.** The runtime holds no state that isn't derivable from the Markdown itself.
- **Your substrate owns history.** Versioning, backup, and undo belong to git, Dropbox, or whatever holds the directory — wikid never touches them. Writes are atomic, last-write-wins.
- **Obsidian-compatible by construction.** YAML frontmatter, `[[wikilinks]]` with aliases, `.obsidian/` ignored. Every feature degrades gracefully when a convention isn't used.
- **Named bearer tokens** for auth. One TOML config: local wikis, client remote profiles, server tokens, bind address.
- **One operation core.** CLI, HTTP, and (next) MCP are thin views over the same operations in `wikid-core` — same behavior and shared JSON wire structs everywhere. Human remote status omits the non-actionable server filesystem root while `status --json` preserves the shared wire struct.

Full spec: [docs/SPEC.md](docs/SPEC.md) · implementation blueprint: [docs/DESIGN.md](docs/DESIGN.md)

## Status

MVP: local mode, the HTTP daemon (`wikid serve`), and remote mode all work and render identically. MCP is the next milestone.

## License

[MIT](LICENSE)
