//! The `rtex` command: typesets a TeX file into PDF.

use std::process::ExitCode;

fn main() -> ExitCode {
    rtex::cli::main()
}
