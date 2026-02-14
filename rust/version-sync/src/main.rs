use clap::Parser;
use anyhow::Result;
use version_sync::VersionSyncer;

#[derive(Parser)]
#[command(name = "sync-version")]
#[command(about = "A high-performance version synchronization tool")]
struct Args {
    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let syncer = VersionSyncer::new();
    let result = syncer.sync_all()?;

    if args.verbose || !result.modified_files.is_empty() {
        println!("Version synced to: {}", result.version);
        if !result.modified_files.is_empty() {
            println!("Modified files: {:?}", result.modified_files);
            println!("Files were modified. Please stage them and commit again.");
            std::process::exit(1); // 让 pre-commit hook 失败，提示重新提交
        } else {
            println!("Version checks passed.");
        }
    }

    Ok(())
}
