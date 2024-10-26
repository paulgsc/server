use std::io;
use tuvitu::progress_bar::ProgressBar;

#[tokio::main]
async fn main() -> io::Result<()> {
	let progress = ProgressBar::new(100).with_message("Processing...").with_width(50);

	for i in 0..=100 {
		progress.set_progress(i).await?;
		tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
	}

	progress.finish_with_message("Done!").await?;
	Ok(())
}
