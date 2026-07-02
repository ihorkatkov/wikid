# wikid

A wicked-simple daemon that exposes plain-Markdown wikis to humans and remote agents.

Point `wikid serve` at a directory of Markdown files — an Obsidian vault works as-is — and every agent on every machine gets fresh, filesystem-feeling access to it over CLI and MCP. No clone, no pull, no push, no database, no git required, no lock-in: the wiki stays plain files the whole time.

```sh
# on the machine that owns the wiki
wikid serve

# from any VM, any agent
wikid grep "auth flow" --wiki projects
wikid cat projects/architecture.md
wikid edit projects/decisions.md
```

## Design

- **Plain files are the product.** The runtime holds no state that isn't derivable from the Markdown itself. Versioning, backup, and undo belong to your substrate (git, Dropbox, whatever) — wikid never touches them.
- **Filesystem-feeling surface.** `ls`, `cat`, `grep`, `glob`, `write`, `edit`, `mv`, `rm`, `links`, `doctor` — the primitives agents already know, over the wire.
- **Agent-optimized output** following the [AXI principles](https://axi.md/): token-efficient, content-first, structured errors, contextual next steps, never interactive.
- **Named bearer tokens** for auth. One TOML config: wikis, tokens, bind address.
- **Dumb runtime.** No LLM inside. Thinking layers on top, through the same public surface.

Full spec: [docs/SPEC.md](docs/SPEC.md)

## Status

Early scaffold. Nothing works yet.

## Development

```sh
cargo build
cargo test
```

Workspace layout:

- `crates/wikid-core` — vault model, link graph, file ops, health checks
- `crates/wikid-server` — HTTP + MCP daemon, auth
- `crates/wikid` — the `wikid` binary (CLI client + `serve`)
