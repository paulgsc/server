/// CLI surface — argument parsing only.
///
/// No I/O, no domain logic, no file access.
/// The sole job of this module is to express what the user typed.
pub mod handlers;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "pocket",
    about = "A small, curated registry of repeatedly useful text fragments",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a new register.
    ///
    /// If VALUE is omitted, reads the value from stdin.
    /// If LABEL is omitted, you will be prompted interactively.
    ///
    /// Examples:
    ///   pocket add "pdf dark mode" "filter: invert(1)..."
    ///   echo "cargo watch -x test" | pocket add "cargo watch"
    Add {
        /// Human-readable label (the thing you will search by).
        label: Option<String>,

        /// The text to store. Omit to read from stdin.
        value: Option<String>,
    },

    /// Open an interactive fuzzy picker and copy the selected value to stdout.
    ///
    /// Compose with xclip/wl-copy/pbcopy:
    ///   pocket query | wl-copy
    Query,

    /// List all registers (labels only).
    #[command(alias = "list")]
    Ls,

    /// Remove a register by exact label.
    ///
    /// Example:
    ///   pocket rm "pdf dark mode"
    Rm {
        /// Exact label of the register to remove.
        label: String,
    },

    /// Edit the value of an existing register in $EDITOR.
    Edit {
        /// Exact label of the register to edit.
        label: String,
    },

    /// Print the current registry file path.
    Path,
}
