mod tui;

use clap::Parser;
use diffsoup::{pr::get_pr_fetcher, repo::open};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "diffsoup")]
#[command(about = "Compare two branches and show interdiff", long_about = None)]
struct Args {
    #[arg(long, value_name = "FROM")]
    from: Option<String>,

    #[arg(long, value_name = "TO")]
    to: Option<String>,

    #[arg(value_name = "PULL REQUEST URL")]
    pr_url: Option<String>,

    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let handle = open(&args.repo)?;
    let workspace = handle.workspace;
    let repo = handle.repo;

    let pr = get_pr_fetcher(args.pr_url, args.from, args.to)?
        .expect("either a PR URL or --from  and --to need to be provided");

    tui::run(workspace, repo, pr)?;

    Ok(())
}
