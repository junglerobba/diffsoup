mod tui;

use clap::Parser;
use diffsoup::repo::open;
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

    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let workspace = open(&args.repo)?;

    let mut app = tui::App::new(workspace)?;
    if let (Some(from), Some(to)) = (&args.from, &args.to) {
        let history = vec![RefNameBuf::from(from), RefNameBuf::from(to)];
        app.set_commit_history(history);
    } else {
        println!("either a PR URL or --from and --to need to be provided");
        process::exit(1);
    }

    app.run()?;

    Ok(())
}
