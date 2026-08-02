// SPDX-License-Identifier: Apache-2.0

//! Offline, provenance-pinned tests over curated upstream XLS examples.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use xlsynth::{
    DslxConvertOptions, IrPackage, IrValue, convert_dslx_to_ir, mangle_dslx_name, optimize_ir,
};
use xlsynth_symex::evaluate;

const MANIFEST: &str = include_str!("corpus/curated/manifest.tsv");
const UPSTREAM_REPOSITORY: &str = "https://github.com/xlsynth/xlsynth";
const UPSTREAM_REVISION: &str = "12bb182e4d842228878d6caf5489df5565c81aa0";

#[derive(Debug)]
struct ManifestEntry<'a> {
    id: &'a str,
    repository: &'a str,
    revision: &'a str,
    source_path: &'a str,
    fixture: &'a str,
    module: &'a str,
    function: &'a str,
    argument_partition: &'a str,
    required_features: &'a str,
    license: &'a str,
}

impl<'a> ManifestEntry<'a> {
    fn parse(line: &'a str) -> Self {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            10,
            "curated corpus manifest row must have 10 tab-separated fields: {line}"
        );
        Self {
            id: fields[0],
            repository: fields[1],
            revision: fields[2],
            source_path: fields[3],
            fixture: fields[4],
            module: fields[5],
            function: fields[6],
            argument_partition: fields[7],
            required_features: fields[8],
            license: fields[9],
        }
    }

    fn source(&self) -> &'static str {
        match self.fixture {
            "tiny_adder.x" => include_str!("corpus/curated/tiny_adder.x"),
            "nested_sel.x" => include_str!("corpus/curated/nested_sel.x"),
            "riscv_simple.x" => include_str!("corpus/curated/riscv_simple.x"),
            fixture => panic!("manifest references unknown curated fixture: {fixture}"),
        }
    }

    fn samples(&self) -> Vec<Vec<(usize, u64)>> {
        match self.id {
            "tiny_adder" => vec![
                vec![(1, 0), (1, 0)],
                vec![(1, 0), (1, 1)],
                vec![(1, 1), (1, 0)],
                vec![(1, 1), (1, 1)],
            ],
            "nested_sel" => vec![
                vec![(8, 0), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 6), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 7), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 33), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 8), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
            ],
            "riscv_decode_opcode" => vec![
                vec![(32, 0)],
                vec![(32, 0b0110011)],
                vec![(32, 0xffff_ffff)],
                vec![(32, 0x1234_56b7)],
            ],
            id => panic!("manifest entry has no deterministic samples: {id}"),
        }
    }
}

fn manifest_entries() -> Vec<ManifestEntry<'static>> {
    MANIFEST
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(ManifestEntry::parse)
        .collect()
}

fn assert_smt_matches_interpreter(
    entry: &ManifestEntry,
    optimization: &str,
    function_name: &str,
    smtlib: &str,
    args: &[(usize, u64)],
    expected_width: usize,
    expected: u64,
) {
    let arguments = args
        .iter()
        .map(|(width, value)| format!(" (_ bv{value} {width})"))
        .collect::<String>();
    let application = format!("(select {function_name}{arguments})");
    let query = format!(
        "{smtlib}\n(assert (not (= {application} (_ bv{expected} {expected_width}))))\n(check-sat)\n"
    );
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
        "{} ({optimization}) z3 failed for arguments {args:?}\nstdout: {}\nstderr: {}\nquery:\n{query}",
        entry.id,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "{} ({optimization}) symbolic result differs from the XLS interpreter for arguments {args:?}\nquery:\n{query}",
        entry.id,
    );
}

fn exercise_package(entry: &ManifestEntry, package: &IrPackage, optimization: &str) {
    let function_name = mangle_dslx_name(entry.module, entry.function)
        .unwrap_or_else(|error| panic!("{}: failed to mangle function name: {error}", entry.id));
    let function = package
        .get_function(&function_name)
        .unwrap_or_else(|error| {
            panic!(
                "{} ({optimization}): function {function_name} is absent: {error}",
                entry.id
            )
        });
    let symbolic = evaluate(&function).unwrap_or_else(|error| {
        panic!(
            "{} ({optimization}): symbolic evaluation failed: {error}",
            entry.id
        )
    });

    for sample in entry.samples() {
        let args: Vec<_> = sample
            .iter()
            .map(|(width, value)| IrValue::make_ubits(*width, *value).unwrap())
            .collect();
        let expected = function.interpret(&args).unwrap_or_else(|error| {
            panic!(
                "{} ({optimization}): XLS interpreter failed for {sample:?}: {error}",
                entry.id
            )
        });
        let expected_width = expected.bit_count().unwrap();
        let expected = expected.to_u64().unwrap();
        assert_smt_matches_interpreter(
            entry,
            optimization,
            &function_name,
            &symbolic.result_smtlib,
            &sample,
            expected_width,
            expected,
        );
    }
}

#[test]
fn curated_manifest_is_valid() {
    let entries = manifest_entries();
    assert!(
        !entries.is_empty(),
        "curated corpus manifest must not be empty"
    );

    for entry in entries {
        assert_eq!(
            entry.repository, UPSTREAM_REPOSITORY,
            "{} repository",
            entry.id
        );
        assert_eq!(entry.revision, UPSTREAM_REVISION, "{} revision", entry.id);
        assert!(
            entry.source_path.starts_with("xls/examples/"),
            "{} must come from the upstream examples corpus",
            entry.id
        );
        assert_eq!(
            entry.argument_partition, "all-symbolic",
            "{} partition",
            entry.id
        );
        assert!(!entry.required_features.is_empty(), "{} features", entry.id);
        assert_eq!(entry.license, "Apache-2.0", "{} license", entry.id);
        assert!(!entry.source().is_empty(), "{} fixture", entry.id);
        assert!(!entry.samples().is_empty(), "{} samples", entry.id);
    }
}

fn exercise_entry(id: &str) {
    let entry = manifest_entries()
        .into_iter()
        .find(|entry| entry.id == id)
        .unwrap_or_else(|| panic!("curated corpus manifest has no {id} entry"));
    let path = Path::new(entry.fixture);
    let converted = convert_dslx_to_ir(entry.source(), path, &DslxConvertOptions::default())
        .unwrap_or_else(|error| panic!("{}: DSLX conversion failed: {error}", entry.id));
    assert!(
        converted.warnings.is_empty(),
        "{}: DSLX conversion emitted warnings: {:?}",
        entry.id,
        converted.warnings
    );

    exercise_package(&entry, &converted.ir, "unoptimized");

    let top = mangle_dslx_name(entry.module, entry.function).unwrap();
    let optimized = optimize_ir(&converted.ir, &top)
        .unwrap_or_else(|error| panic!("{}: IR optimization failed: {error}", entry.id));
    exercise_package(&entry, &optimized, "optimized");
}

#[test]
fn tiny_adder_matches_xls_interpreter() {
    exercise_entry("tiny_adder");
}

#[test]
fn nested_sel_matches_xls_interpreter() {
    exercise_entry("nested_sel");
}

#[test]
fn riscv_decode_opcode_matches_xls_interpreter() {
    exercise_entry("riscv_decode_opcode");
}
