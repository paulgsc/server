use noobgit::NoobGit;
use std::path::PathBuf;
use file_reader::core::Path as ValidatedPath;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let user_input_path = "/path/to/watch"; 

    match ValidatedPath::parse(user_input_path) {
        Ok(valid_path) => {
            println!("path validated: {:?}", valid_path);
            let path_buf = PathBuf::from(valid_path.as_ref());
            watch_directory(path_buf).await?;
        },
        Err(e) => {
            eprintln!("invalid path: {:?}", e);
            return Err(Box::new(e) as Box<dyn std::error::Error>);
        }
    }

    Ok(())
}

async fn watch_directory(root_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    println!("watching directory: {:?}", root_path);

    let mut noob_git = NoobGit::new(&root_path).await?;

    noob_git.start_watching().await?;

    loop {
        let notifications = noob_git.get_notifications();
        if !notifications.is_empty() {
            println!("notifications:");
            for notification in notifications {
                println!("  {}", notification);
            }
        }

    }
}

