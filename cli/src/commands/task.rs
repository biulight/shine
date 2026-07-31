use clap::{Args, Subcommand, ValueHint};
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum TaskCommands {
    /// Save a command as a named task
    Save {
        /// Task name (letters, numbers, dots, dashes, underscores)
        name: String,
        /// Replace an existing task with the same name
        #[arg(long)]
        force: bool,
        /// Always run the task from this directory
        #[arg(long, value_hint = ValueHint::DirPath)]
        cwd: Option<PathBuf>,
        /// Command and arguments to save (after `--`)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Run a saved task in the current directory
    Run(TaskRunCommand),
    /// List saved tasks
    List,
    /// Show a saved task's full command
    Info {
        /// Task name
        name: String,
    },
    /// Delete a saved task
    Delete {
        /// Task name
        name: String,
    },
}

#[derive(Args, Debug)]
pub struct TaskRunCommand {
    /// Task name
    pub name: String,
    /// Extra arguments appended to the saved command (after `--`)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub extra: Vec<String>,
}
