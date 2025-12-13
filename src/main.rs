mod tui;

use clap::Parser;
use diffsoup::{
    pr::{PrFetcher, get_pr_fetcher},
    repo::{ensure_commits_exist, open},
};
use jj_lib::ref_name::RefNameBuf;
use std::{
    path::PathBuf,
    process::{self},
};

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

    let workspace = open(&args.repo)?;
    let repo = workspace.repo_loader().load_at_head()?;

    let pr = args
        .pr_url
        .map(|url| get_pr_fetcher(&url))
        .unwrap_or(Ok(None))?;
    let (repo, commits, from, to) =
        if let Some(history) = pr.as_deref().map(PrFetcher::fetch_history) {
            let commits = history?.0;
            if commits.len() < 2 {
                println!("Not enough commits found in PR");
                process::exit(1);
            }
            let repo = ensure_commits_exist(commits.iter(), repo.clone())?;

            let from = if let Some(from) = &args.from
                && let Some(index) = commits.iter().position(|c| c.as_str() == from)
            {
                index
            } else {
                0
            };
            let to = if let Some(to) = &args.to
                && let Some(index) = commits.iter().position(|c| c.as_str() == to)
            {
                index
            } else {
                commits.len() - 1
            };
            (repo, commits, from, to)
        } else if let (Some(from), Some(to)) = (&args.from, &args.to) {
            let commits = vec![RefNameBuf::from(from), RefNameBuf::from(to)];
            (repo, commits, 0, 1)
        } else {
            println!("either a PR URL or --from and --to need to be provided");
            process::exit(1);
        };

    let mut app = tui::App::new(workspace, repo)?;
    app.set_commit_history(commits);
    app.set_base_index(from);
    app.set_comparison_index(to);

    app.run()?;

    Ok(())
}
