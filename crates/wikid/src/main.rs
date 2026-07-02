use clap::{Parser, Subcommand};

/// wikid — plain-Markdown wikis for humans and remote agents.
///
/// Point `wikid serve` at one or more wiki directories (Obsidian vaults
/// included) and every agent gets filesystem-feeling access over CLI and MCP.
#[derive(Parser)]
#[command(name = "wikid", version, about, arg_required_else_help = false)]
struct Cli {
	#[command(subcommand)]
	command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
	/// Run the daemon serving configured wikis
	Serve,
	/// Show wikis, page counts, recent activity, and health summary
	Status,
	/// List pages and directories
	Ls { path: Option<String> },
	/// Read a page
	Cat { path: String },
	/// Search page content
	Grep { pattern: String },
	/// Find pages by path pattern
	Glob { pattern: String },
	/// Create or overwrite a page
	Write { path: String },
	/// Surgically edit a page (string replace)
	Edit { path: String },
	/// Rename or move a page
	Mv { from: String, to: String },
	/// Delete a page (requires --force)
	Rm {
		path: String,
		#[arg(long)]
		force: bool,
	},
	/// Show outgoing links and backlinks for a page
	Links { path: String },
	/// Run structural health checks
	Doctor,
}

fn main() {
	let cli = Cli::parse();
	// AXI principle 8: no arguments shows live data, not help text.
	let _command = cli.command.unwrap_or(Command::Status);
	// Structured errors go to stdout; exit code 1 for errors.
	println!("error: not implemented yet (scaffold) — see docs/SPEC.md");
	std::process::exit(1);
}
