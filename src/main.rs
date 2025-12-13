mod tui;

use clap::Parser;
use diffsoup::repo::open;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "diffsoup")]
#[command(about = "Compare two branches and show interdiff", long_about = None)]
struct Args {
    // TODO make these optional and implement picker
    #[arg(value_name = "FROM")]
    from: String,

    #[arg(value_name = "TO")]
    to: String,

    #[arg(short, long, default_value = ".")]
    repo: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let workspace = open(&args.repo)?;

    let mut app = tui::App::new(workspace)?;
    app.set_base_branch(&args.from);
    app.set_comparison_branch(&args.to);

    app.run()?;

    Ok(())
}
