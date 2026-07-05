# wikid

[![CI](https://github.com/ihorkatkov/wikid/actions/workflows/ci.yml/badge.svg)](https://github.com/ihorkatkov/wikid/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ihorkatkov/wikid)](https://github.com/ihorkatkov/wikid/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

**Give every coding agent on every machine one shared, plain-Markdown knowledge base.**

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

From any VM, any agent — same commands, same output, over HTTP:

```sh
export WIKID_SERVER=http://127.0.0.1:7448
export WIKID_TOKEN=...
export WIKID_WIKI=notes

wikid status
wikid grep "auth flow"
wikid cat architecture.md
wikid cat decisions.md --hashes            # each line as line:hash: text
wikid edit decisions.md --line 4 --hash 3b39a78cfdcb --new "status: final"
wikid edit decisions.md --line 5 --hash 9a1b2c3d4e5f --new="- status starts with dash"
printf '%s' '[{"line":4,"expected_hash":"3b39a78cfdcb","new_text":"status: final"}]' \
  | wikid edit-batch decisions.md
```

In remote mode, `status` labels the root as `root (server): ...`. That path lives on the machine running `wikid serve`; use `wikid cat`/`grep`/`edit` from the client, not local shell file commands against that path.

(`--server`, `--token`, and `--wiki` flags work too. Network exposure is your choice: localhost, tailscale, or public + TLS.)

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

# Fallback wiki for zero-target local commands run outside any registered wiki.
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
```

On the client side, every remote setting is a flag with an env-var twin, so you can bake a connection into an agent VM's environment once:

| Flag | Env var | Meaning |
|---|---|---|
| `--server` | `WIKID_SERVER` | Remote daemon URL |
| `--token` | `WIKID_TOKEN` | Bearer token |
| `--wiki` | `WIKID_WIKI` | Wiki name on the daemon |
| `--dir` | `WIKID_DIR` | Local directory (local mode) |
| `--config` | `WIKID_CONFIG` | Config file path |

With none of these set, wikid reads config and picks the wiki containing the current directory, the only registered wiki, or `default_wiki`.

## The surface

Every command works identically in local and remote mode, and every command takes `--json`:

| Command | What it does |
|---|---|
| `status` | Page counts, recent activity, health summary — the no-arg default |
| `ls` / `tree` / `glob` | Find pages by path |
| `cat` | Read a page (large files truncated with a size hint; `--full` to override) |
| `grep` | Regex search with ranked results and match context |
| `write` / `edit` / `edit-batch` | Create pages; hash-guarded line edits — a stale hash refuses the whole edit batch, so concurrent writers never silently clobber each other |
| `mv` / `rm` | Rename and delete (`rm` requires `--force` — never an interactive prompt) |
| `links` | Outgoing links and backlinks from the wikilink graph |
| `doctor` | Structural health checks: broken wikilinks, orphans, stale and oversized pages |
| `update` | Explicitly update the installed local `wikid` binary from verified GitHub release assets |

Output follows the [AXI principles](https://axi.md/) for agent-facing CLIs: token-efficient, content-first, structured errors on stdout, exit codes 0/1/2, contextual next-step hints, never interactive.

## Giving your agents the wiki

Because the surface is just a CLI, wiring an agent up is a paragraph in your `CLAUDE.md` / `AGENTS.md`, not a plugin:

```markdown
## Shared wiki

A team knowledge base is available via the `wikid` CLI ($WIKID_SERVER,
$WIKID_TOKEN, $WIKID_WIKI are set). Before starting work, `wikid grep`
for prior notes on the topic. Record durable findings with `wikid write`
/ `wikid edit`, and link related pages with [[wikilinks]].
```

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
- **Named bearer tokens** for auth. One TOML config: wikis, tokens, bind address.
- **One operation core.** CLI, HTTP, and (next) MCP are thin views over the same operations in `wikid-core` — same behavior and shared JSON wire structs everywhere. Human remote `status` labels `root` as server-side so agents do not mistake it for a local path.

Full spec: [docs/SPEC.md](docs/SPEC.md) · implementation blueprint: [docs/DESIGN.md](docs/DESIGN.md)

## Status

MVP: local mode, the HTTP daemon (`wikid serve`), and remote mode all work and render identically. MCP is the next milestone.

## License

[MIT](LICENSE)
