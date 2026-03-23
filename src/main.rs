mod commands;

use anyhow::Result;
use clap::Parser;
use commands::pm::Cli;

fn main() -> Result<()> {
    let cli = Cli::parse();
    commands::pm::run(cli)
}
