//! Prints every place in the workspace where control flow nests too deep.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let violations = match style_check::check_dir(&root.join("crates")) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("style-check: {e}");
            return ExitCode::FAILURE;
        }
    };
    for v in &violations {
        println!("{v}");
    }
    if violations.is_empty() {
        return ExitCode::SUCCESS;
    }
    eprintln!("{} places nest too deep", violations.len());
    ExitCode::FAILURE
}
