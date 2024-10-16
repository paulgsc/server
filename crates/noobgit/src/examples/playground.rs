use mini_git::MiniGit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut git = MiniGit::new("simple-git-playground")?;
    git.start_watching()?;
    Ok(())
}
