// SPDX-License-Identifier: Apache-2.0

//! Enforces repository-wide source conventions inherited from xlsynth-crate.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const SPDX_IDENTIFIER: &str = "SPDX-License-Identifier: Apache-2.0";

fn repository_files() -> BTreeSet<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut pending = vec![root.to_owned()];
    let mut files = BTreeSet::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("repository directory must be readable") {
            let entry = entry.expect("repository entry must be readable");
            let path = entry.path();
            if path.is_dir() {
                if !matches!(
                    entry.file_name().to_str(),
                    Some(".cache" | ".codex" | ".git" | "target")
                ) {
                    pending.push(path);
                }
            } else if maintained_text_file(&path) {
                files.insert(path);
            }
        }
    }
    files
}

fn maintained_text_file(path: &Path) -> bool {
    if matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".dockerignore" | ".gitignore" | "Dockerfile" | "LICENSE")
    ) {
        return true;
    }
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("json" | "md" | "rs" | "sh" | "svg" | "toml" | "tsv" | "x" | "yaml" | "yml")
    )
}

fn requires_spdx(path: &Path) -> bool {
    // Curated `.x` fixtures are byte-for-byte upstream copies with their
    // original Apache-2.0 notices and manifest provenance.
    if path.extension().and_then(|extension| extension.to_str()) == Some("x")
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            == Some("curated")
    {
        return false;
    }
    path.file_name().and_then(|name| name.to_str()) == Some("Dockerfile")
        || matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("json" | "rs" | "sh" | "svg" | "tsv" | "x" | "yaml" | "yml")
        )
}

fn has_spdx(path: &Path, contents: &str) -> bool {
    if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
        return contents.contains(&format!("\"_spdx\": \"{SPDX_IDENTIFIER}\""));
    }
    contents
        .lines()
        .take(2)
        .any(|line| line.contains(SPDX_IDENTIFIER))
}

#[test]
fn maintained_sources_have_spdx_headers() {
    let missing = repository_files()
        .into_iter()
        .filter(|path| requires_spdx(path))
        .filter(|path| {
            let contents = fs::read_to_string(path).expect("maintained source must be UTF-8");
            !has_spdx(path, &contents)
        })
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing SPDX identifiers: {missing:#?}");
}

#[test]
fn maintained_svg_assets_are_accessible() {
    let invalid = repository_files()
        .into_iter()
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("svg"))
        .filter(|path| {
            let contents = fs::read_to_string(path).expect("maintained SVG must be UTF-8");
            [
                "<title id=",
                "<desc id=",
                "role=\"img\"",
                "aria-labelledby=",
            ]
            .into_iter()
            .any(|marker| !contents.contains(marker))
        })
        .collect::<Vec<_>>();
    assert!(
        invalid.is_empty(),
        "SVG assets without accessible metadata: {invalid:#?}"
    );
}

#[test]
fn workflow_filenames_match_workflow_names() {
    let workflow_directory = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut mismatches = Vec::new();
    for entry in fs::read_dir(workflow_directory).expect("workflow directory must be readable") {
        let path = entry.expect("workflow entry must be readable").path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yaml" | "yml")
        ) {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("workflow must be UTF-8");
        let workflow_name = contents
            .lines()
            .find_map(|line| line.strip_prefix("name: "))
            .expect("workflow must have a top-level name");
        let expected_stem = workflow_name.to_ascii_lowercase().replace(' ', "-");
        if path.file_stem().and_then(|stem| stem.to_str()) != Some(expected_stem.as_str()) {
            mismatches.push((path, workflow_name.to_owned()));
        }
    }
    assert!(
        mismatches.is_empty(),
        "workflow filenames must match normalized workflow names: {mismatches:#?}"
    );
}

#[test]
fn maintained_text_is_canonical() {
    let mut missing_final_newline = Vec::new();
    let mut trailing_whitespace = Vec::new();
    for path in repository_files() {
        let contents = fs::read_to_string(&path).expect("maintained source must be UTF-8");
        if !contents.is_empty() && !contents.ends_with('\n') {
            missing_final_newline.push(path.clone());
        }
        for (index, line) in contents.lines().enumerate() {
            if line.ends_with([' ', '\t']) {
                trailing_whitespace.push((path.clone(), index + 1));
            }
        }
    }
    assert!(
        missing_final_newline.is_empty(),
        "files without final newlines: {missing_final_newline:#?}"
    );
    assert!(
        trailing_whitespace.is_empty(),
        "lines with trailing whitespace: {trailing_whitespace:#?}"
    );
}

#[test]
fn maintained_text_has_no_machine_specific_paths() {
    let forbidden = [concat!("/", "Users", "/"), concat!("/", "home", "/")];
    let violations = repository_files()
        .into_iter()
        .filter(|path| {
            let contents = fs::read_to_string(path).expect("maintained source must be UTF-8");
            forbidden.iter().any(|prefix| contents.contains(prefix))
        })
        .collect::<Vec<_>>();
    assert!(
        violations.is_empty(),
        "machine-specific absolute paths: {violations:#?}"
    );
}
