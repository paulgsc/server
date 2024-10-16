use noobgit::NoobGit;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut git = NoobGit::new("simple-git-playground")?;
    git.start_watching()?;
    Ok(())
}
