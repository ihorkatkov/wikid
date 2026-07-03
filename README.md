# wikid

A wicked-simple daemon that exposes plain-Markdown wikis to humans and remote agents.

Point `wikid serve` at a directory of Markdown files — an Obsidian vault works as-is — and every agent on every machine gets fresh, filesystem-feeling access to it over CLI and HTTP (MCP next). No clone, no pull, no push, no database, no git required, no lock-in: the wiki stays plain files the whole time.

## Quickstart

```sh
cargo install --path crates/wikid
```

### First run

Create a blank Karpathy-style LLM Wiki scaffold and register it in config:

```sh
wikid init ~/notes
wikid status                 # no --dir needed when config can identify the wiki
```

`init` creates `index.md`, `log.md`, `AGENTS.md`, and `raw/`, `raw/assets/`, `concepts/`, `entities/`, `questions/`, `syntheses/`. It never overwrites existing files, so it is safe in a non-empty directory or existing Obsidian vault.

### Local mode

Any directory of Markdown files is already a wiki. Target explicitly, via env, or via config fallback:

```sh
wikid --dir ~/notes status
export WIKID_DIR=~/notes
wikid grep "auth flow"
wikid cat projects/architecture.md
```

When neither `--dir` nor remote flags are set, wikid reads config (`--config`, `$WIKID_CONFIG`, `./wikid.toml`, then `~/.config/wikid/config.toml`) and chooses the wiki containing the current directory, the only registered wiki, or `default_wiki`.

Every command takes `--json` to emit the result as one JSON object.

### Serve + remote mode

On the machine that owns the wiki, start the daemon. If no config exists, `serve` creates `~/.config/wikid/config.toml`, registers the current directory, generates an admin token, prints where it was written (not the secret), then serves immediately:

```sh
cd ~/notes
wikid serve
wikid token show admin       # explicit secret-revealing command
```

You can also maintain config manually; see [docs/wikid.example.toml](docs/wikid.example.toml).

From any VM, any agent — same commands, same output, over HTTP:

```sh
export WIKID_SERVER=http://127.0.0.1:7448
export WIKID_TOKEN=$(wikid token show admin | head -1)
export WIKID_WIKI=notes

wikid status
wikid grep "auth flow"
wikid cat architecture.md
wikid edit decisions.md --old "status: draft" --new "status: final"
```

(`--server`, `--token`, and `--wiki` flags work too.)

## Design

- **Plain files are the product.** The runtime holds no state that isn't derivable from the Markdown itself. Versioning, backup, and undo belong to your substrate (git, Dropbox, whatever) — wikid never touches them.
- **Filesystem-feeling surface.** `ls`, `cat`, `grep`, `glob`, `write`, `edit`, `mv`, `rm`, `links`, `doctor` — the primitives agents already know, over the wire.
- **Agent-optimized output** following the [AXI principles](https://axi.md/): token-efficient, content-first, structured errors, contextual next steps, never interactive.
- **Named bearer tokens** for auth. One TOML config: wikis, tokens, bind address.
- **Dumb runtime.** No LLM inside. Thinking layers on top, through the same public surface.

## Example wiki

`examples/llm-wiki/` is a small public-safe demo vault inspired by
[Andrej Karpathy's LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f).
It shows a persistent LLM-maintained knowledge base with raw-source intake,
compiled concept pages, workflows, an index, a log, and clean wikilinks:

```sh
wikid --dir examples/llm-wiki status
wikid --dir examples/llm-wiki links index.md
wikid --dir examples/llm-wiki doctor
```

Full spec: [docs/SPEC.md](docs/SPEC.md) · implementation blueprint: [docs/DESIGN.md](docs/DESIGN.md)

## Status

MVP: local mode, the HTTP daemon (`wikid serve`), and remote mode all work and
render identically. MCP is the next milestone.

## Development

```sh
cargo build
cargo test
```

### Boxd worktrees

`boxd-worktree` creates a fast, deterministic boxd fork from the warm `wikid-golden` VM and checks out a new branch inside the fork:

```sh
cargo run --bin boxd-worktree -- create neo-123-demo
cargo run --bin boxd-worktree -- --json create neo-123-demo --name wikid-neo-123-demo
```

It fails if the target VM or branch already exists, destroys a partially-created fork on setup failure, restarts `wikid serve`, and prints the fork URL plus SSH command.

Workspace layout:

- `crates/wikid-core` — vault model, link graph, file ops, health checks
- `crates/wikid-server` — HTTP daemon (MCP later), bearer-token auth
- `crates/wikid` — the `wikid` binary (CLI, remote client, `serve`)
