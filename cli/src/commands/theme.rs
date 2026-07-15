use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum ThemeCommands {
    /// Resolve the terminal's light/dark theme and print shell export
    /// statements for SHINE_TERMINAL_THEME and BAT_THEME
    Sync {
        /// Gate on the sync_terminal_theme config toggle and
        /// SHINE_SYNC_TERMINAL_THEME env var (used by the managed profile's
        /// automatic call). Manual invocations omit this and always run.
        #[arg(long)]
        auto: bool,
        /// Suppress non-essential diagnostics on stderr
        #[arg(long)]
        quiet: bool,
    },
}
