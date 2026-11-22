//! Runs the house-style checks over the whole workspace.

use std::path::Path;

#[test]
fn control_flow_in_the_workspace_is_flat() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let violations = style_check::check_dir(&crates).unwrap();
    let report: Vec<String> = violations.iter().map(|v| v.to_string()).collect();
    assert!(
        report.is_empty(),
        "control flow nests too deep in {} places:\n{}",
        report.len(),
        report.join("\n"),
    );
}
