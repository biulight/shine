use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CheckCommands {
    /// Check which app config files are applied locally
    App,
    /// Check which shell presets are installed locally
    Shell,
}
