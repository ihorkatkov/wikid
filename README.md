# wikid

A wicked-simple daemon that exposes plain-Markdown wikis to humans and remote agents.

Point `wikid serve` at a directory of Markdown files — an Obsidian vault works as-is — and every agent on every machine gets fresh, filesystem-feeling access to it over CLI and HTTP (MCP next). No clone, no pull, no push, no database, no git required, no lock-in: the wiki stays plain files the whole time.

## Quickstart

```sh
cargo install --path crates/wikid
```

### Local mode

Any directory of Markdown files is already a wiki:

```sh
wikid --dir ~/notes            # no-arg default = status: pages, recent, health
wikid --dir ~/notes ls
wikid --dir ~/notes grep "auth flow"
wikid --dir ~/notes cat projects/architecture.md
```

Set `WIKID_DIR` once and drop the flag:

```sh
export WIKID_DIR=~/notes
wikid write projects/decisions.md -m "# Decisions"
wikid edit projects/decisions.md --old "# Decisions" --new "# Decisions (draft)"
wikid links projects/decisions.md
wikid rm projects/decisions.md --force
wikid doctor
```

Every command takes `--json` to emit the result as one JSON object.

### Serve + remote mode

On the machine that owns the wiki, write a config (see
[docs/wikid.example.toml](docs/wikid.example.toml)) and start the daemon:

```sh
cat > wikid.toml <<'EOF'
bind = "127.0.0.1:7448"

[wikis]
projects = "/home/you/wikis/projects"

[tokens]
"wkd_change_me" = "agent-vm-1"
EOF

wikid serve    # config discovery: --config → $WIKID_CONFIG → ./wikid.toml → ~/.config/wikid/config.toml
```

From any VM, any agent — same commands, same output, over HTTP:

```sh
export WIKID_SERVER=http://127.0.0.1:7448
export WIKID_TOKEN=wkd_change_me
export WIKID_WIKI=projects

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

Full spec: [docs/SPEC.md](docs/SPEC.md) · implementation blueprint: [docs/DESIGN.md](docs/DESIGN.md)

## Status

MVP: local mode, the HTTP daemon (`wikid serve`), and remote mode all work and
render identically. MCP is the next milestone.

## Development

```sh
cargo build
cargo test
```

Workspace layout:

- `crates/wikid-core` — vault model, link graph, file ops, health checks
- `crates/wikid-server` — HTTP daemon (MCP later), bearer-token auth
- `crates/wikid` — the `wikid` binary (CLI, remote client, `serve`)
