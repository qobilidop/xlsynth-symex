// SPDX-License-Identifier: Apache-2.0

//! Shared integration-test support.

use std::io::Write;
use std::process::{Command, Stdio};

/// Runs one generated SMT-LIB query through the development image's Z3.
pub fn run_z3(query: &str, context: &str) -> String {
    let mut child = Command::new("z3")
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("z3 must be present in the development image");
    child
        .stdin
        .take()
        .expect("z3 stdin must be piped")
        .write_all(query.as_bytes())
        .expect("SMT query must be writable");
    let output = child.wait_with_output().expect("z3 must finish");
    assert!(
        output.status.success(),
        "{context}: z3 failed\nstdout: {}\nstderr: {}\nquery:\n{query}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("z3 stdout must be UTF-8")
        .trim()
        .to_owned()
}
