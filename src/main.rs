mod tui;

use clap::Parser;
use diffsoup::{
    pr::{PrFetcher, get_pr_fetcher},
    repo::open,
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

    let mut app = tui::App::new(workspace)?;
    let pr = args
        .pr_url
        .map(|url| get_pr_fetcher(&url))
        .unwrap_or(Ok(None))?;
    if let Some(history) = pr.as_deref().map(PrFetcher::fetch_history) {
        let commits = history?.0;
        if commits.len() < 2 {
            println!("Not enough commits found in PR");
            process::exit(1);
        }
        app.set_comparison_index(commits.len() - 1);
        app.set_commit_history(commits);
    } else if let (Some(from), Some(to)) = (&args.from, &args.to) {
        let history = vec![RefNameBuf::from(from), RefNameBuf::from(to)];
        app.set_commit_history(history);
    } else {
        println!("either a PR URL or --from and --to need to be provided");
        process::exit(1);
    }

    app.run()?;

    Ok(())
}
