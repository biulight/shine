use anyhow::Result;

use crate::{colors, config::Config};

pub async fn handle_init(yes: bool) -> Result<()> {
    let current_dir = std::env::current_dir()?;
    let display_dir = tokio::fs::canonicalize(&current_dir)
        .await
        .unwrap_or(current_dir);

    if !yes && !confirm_init(&display_dir)? {
        println!("{}", colors::dim("Init cancelled."));
        return Ok(());
    }

    let path = Config::init_current_dir_config().await?;
    println!(
        "{}",
        colors::green(&format!("Initialized shine config at {}", path.display()))
    );
    println!(
        "{}",
        colors::dim(&format!("presets_dir = {}", display_dir.display()))
    );
    Ok(())
}

fn confirm_init(dir: &std::path::Path) -> Result<bool> {
    use std::io::Write as _;

    print!(
        "Initialize {} as the shine presets directory? [y/N] ",
        dir.display()
    );
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let answer = input.trim();
    Ok(answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes"))
}
