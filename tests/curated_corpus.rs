// SPDX-License-Identifier: Apache-2.0

//! Offline, provenance-pinned validation of curated upstream XLS examples.

use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use xlsynth::{
    DslxConvertOptions, IrFunction, IrPackage, IrValue, convert_dslx_to_ir, mangle_dslx_name,
    optimize_ir,
};
use xlsynth_symex::{SymexResult, evaluate_package};

const MANIFEST: &str = include_str!("corpus/curated/manifest.tsv");
const VALIDATION_MATRIX: &str = include_str!("corpus/curated/validation.tsv");
const UPSTREAM_REPOSITORY: &str = "https://github.com/xlsynth/xlsynth";
const UPSTREAM_REVISION: &str = "12bb182e4d842228878d6caf5489df5565c81aa0";
const PATH_WITNESS_REPLAY_BLOCKER: &str = "blocked:selection-traces";

type BitsInput = Vec<(usize, u64)>;

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
    fuzz_cases: usize,
    fuzz_seed: u64,
}

impl<'a> ManifestEntry<'a> {
    fn parse(line: &'a str) -> Self {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            12,
            "curated corpus manifest row must have 12 tab-separated fields: {line}"
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
            fuzz_cases: fields[10].parse().unwrap_or_else(|error| {
                panic!("{} has invalid fuzz case count: {error}", fields[0])
            }),
            fuzz_seed: u64::from_str_radix(fields[11], 16)
                .unwrap_or_else(|error| panic!("{} has invalid fuzz seed: {error}", fields[0])),
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

    fn input_widths(&self) -> &'static [usize] {
        match self.id {
            "tiny_adder" => &[1, 1],
            "nested_sel" => &[8, 8, 8, 8, 8, 8],
            "riscv_decode_opcode" => &[32],
            id => panic!("manifest entry has no input shape: {id}"),
        }
    }

    fn curated_vectors(&self) -> Vec<BitsInput> {
        match self.id {
            "tiny_adder" => vec![
                vec![(1, 0), (1, 0)],
                vec![(1, 0), (1, 1)],
                vec![(1, 1), (1, 0)],
                vec![(1, 1), (1, 1)],
            ],
            "nested_sel" => vec![
                vec![(8, 0), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 3), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 4), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 6), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 7), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 8), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 32), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 33), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
                vec![(8, 255), (8, 11), (8, 22), (8, 33), (8, 44), (8, 55)],
            ],
            "riscv_decode_opcode" => vec![
                vec![(32, 0)],
                vec![(32, 0b0000011)],
                vec![(32, 0b0010011)],
                vec![(32, 0b0110011)],
                vec![(32, 0b1100011)],
                vec![(32, 0xffff_ffff)],
                vec![(32, 0x1234_56b7)],
            ],
            id => panic!("manifest entry has no curated vectors: {id}"),
        }
    }

    fn fuzz_vectors(&self) -> Vec<BitsInput> {
        if self.id == "tiny_adder" {
            return (0..self.fuzz_cases)
                .map(|case| vec![(1, (case & 1) as u64), (1, ((case >> 1) & 1) as u64)])
                .collect();
        }

        let mut rng = DeterministicRng::new(self.fuzz_seed);
        (0..self.fuzz_cases)
            .map(|_| {
                self.input_widths()
                    .iter()
                    .map(|width| (*width, rng.next() & bit_mask(*width)))
                    .collect()
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum IrForm {
    Unoptimized,
    Optimized,
}

impl IrForm {
    const ALL: [Self; 2] = [Self::Unoptimized, Self::Optimized];

    const fn name(self) -> &'static str {
        match self {
            Self::Unoptimized => "unoptimized",
            Self::Optimized => "optimized",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "unoptimized" => Self::Unoptimized,
            "optimized" => Self::Optimized,
            value => panic!("unknown IR form in validation matrix: {value}"),
        }
    }
}

#[derive(Debug)]
struct ValidationEntry<'a> {
    id: &'a str,
    ir_form: IrForm,
    curated_vector_differential: &'a str,
    differential_fuzz: &'a str,
    symbolic_equivalence: &'a str,
    path_witness_replay: &'a str,
}

impl<'a> ValidationEntry<'a> {
    fn parse(line: &'a str) -> Self {
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            6,
            "validation matrix row must have 6 tab-separated fields: {line}"
        );
        Self {
            id: fields[0],
            ir_form: IrForm::parse(fields[1]),
            curated_vector_differential: fields[2],
            differential_fuzz: fields[3],
            symbolic_equivalence: fields[4],
            path_witness_replay: fields[5],
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum DifferentialMode {
    CuratedVector,
    Fuzz,
}

impl DifferentialMode {
    const fn name(self) -> &'static str {
        match self {
            Self::CuratedVector => "curated-vector differential",
            Self::Fuzz => "differential fuzz",
        }
    }
}

struct DeterministicRng(u64);

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const fn bit_mask(width: usize) -> u64 {
    if width == 64 {
        u64::MAX
    } else {
        (1_u64 << width) - 1
    }
}

fn data_lines(data: &'static str) -> impl Iterator<Item = &'static str> {
    data.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

fn manifest_entries() -> Vec<ManifestEntry<'static>> {
    data_lines(MANIFEST).map(ManifestEntry::parse).collect()
}

fn validation_entries() -> Vec<ValidationEntry<'static>> {
    data_lines(VALIDATION_MATRIX)
        .map(ValidationEntry::parse)
        .collect()
}

fn compile_entry(entry: &ManifestEntry) -> (IrPackage, IrPackage, String) {
    let converted = convert_dslx_to_ir(
        entry.source(),
        Path::new(entry.fixture),
        &DslxConvertOptions::default(),
    )
    .unwrap_or_else(|error| panic!("{}: DSLX conversion failed: {error}", entry.id));
    assert!(
        converted.warnings.is_empty(),
        "{}: DSLX conversion emitted warnings: {:?}",
        entry.id,
        converted.warnings
    );
    let function_name = mangle_dslx_name(entry.module, entry.function)
        .unwrap_or_else(|error| panic!("{}: failed to mangle function name: {error}", entry.id));
    let optimized = optimize_ir(&converted.ir, &function_name)
        .unwrap_or_else(|error| panic!("{}: IR optimization failed: {error}", entry.id));
    (converted.ir, optimized, function_name)
}

fn assert_differential_samples(
    entry: &ManifestEntry,
    ir_form: IrForm,
    mode: DifferentialMode,
    function: &IrFunction,
    symbolic: &SymexResult,
    samples: &[BitsInput],
) {
    assert!(
        !samples.is_empty(),
        "{}: no {} samples",
        entry.id,
        mode.name()
    );
    let mut mismatches = Vec::with_capacity(samples.len());
    for (case, sample) in samples.iter().enumerate() {
        let args: Vec<_> = sample
            .iter()
            .map(|(width, value)| IrValue::make_ubits(*width, *value).unwrap())
            .collect();
        let expected = function.interpret(&args).unwrap_or_else(|error| {
            panic!(
                "{} ({}, {} case {case}): XLS interpreter failed for {sample:?}: {error}",
                entry.id,
                ir_form.name(),
                mode.name()
            )
        });
        let expected_width = expected.bit_count().unwrap();
        let expected = expected.to_u64().unwrap_or_else(|error| {
            panic!(
                "{} ({}, {} case {case}): result does not fit the bits-only harness: {error}",
                entry.id,
                ir_form.name(),
                mode.name()
            )
        });
        assert_eq!(symbolic.parameters.len(), sample.len());
        let bindings = symbolic
            .parameters
            .iter()
            .zip(sample)
            .map(|(parameter, (width, value))| {
                assert_eq!(parameter.bit_count, *width);
                format!("({} (_ bv{value} {width}))", parameter.name)
            })
            .collect::<Vec<_>>()
            .join(" ");
        mismatches.push(format!(
            "(let ({bindings}) (not (= {} (_ bv{expected} {expected_width}))))",
            symbolic.result.expression
        ));
    }

    let query = format!(
        "(assert (or\n  {}))\n(check-sat)\n",
        mismatches.join("\n  ")
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
        "{} ({}, {}) z3 failed for {} samples\nstdout: {}\nstderr: {}",
        entry.id,
        ir_form.name(),
        mode.name(),
        samples.len(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "{} ({}) failed {} testing over {} samples (seed {:016x})",
        entry.id,
        ir_form.name(),
        mode.name(),
        samples.len(),
        entry.fuzz_seed,
    );
}

fn run_differential_validation(mode: DifferentialMode) {
    for entry in manifest_entries() {
        let samples = match mode {
            DifferentialMode::CuratedVector => entry.curated_vectors(),
            DifferentialMode::Fuzz => entry.fuzz_vectors(),
        };
        let (unoptimized, optimized, function_name) = compile_entry(&entry);
        for (ir_form, package) in [
            (IrForm::Unoptimized, &unoptimized),
            (IrForm::Optimized, &optimized),
        ] {
            let function = package
                .get_function(&function_name)
                .unwrap_or_else(|error| {
                    panic!(
                        "{} ({}): function {function_name} is absent: {error}",
                        entry.id,
                        ir_form.name()
                    )
                });
            let symbolic = evaluate_package(package, &function_name).unwrap_or_else(|error| {
                panic!(
                    "{} ({}): symbolic evaluation failed during {} testing: {error}",
                    entry.id,
                    ir_form.name(),
                    mode.name()
                )
            });
            assert_differential_samples(&entry, ir_form, mode, &function, &symbolic, &samples);
        }
    }
}

fn assert_symbolic_equivalence(
    entry: &ManifestEntry,
    ir_form: IrForm,
    function: &IrFunction,
    symbolic: &SymexResult,
    function_name: &str,
) {
    let reference = function.to_z3_smtlib().unwrap_or_else(|error| {
        panic!(
            "{} ({}): XLS reference SMT translation failed: {error}",
            entry.id,
            ir_form.name()
        )
    });
    let arguments = symbolic
        .parameters
        .iter()
        .map(|parameter| format!(" {}", parameter.name))
        .collect::<String>();
    let query = format!(
        "{}\n{reference}\n(assert (not (= xlsynth_symex_result (select {function_name}{arguments}))))\n(check-sat)\n",
        symbolic.result_smtlib
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
        .expect("equivalence query must be writable");
    let output = child.wait_with_output().expect("z3 must finish");
    assert!(
        output.status.success(),
        "{} ({}) symbolic equivalence solver failed\nstdout: {}\nstderr: {}\nquery:\n{query}",
        entry.id,
        ir_form.name(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "{} ({}) native result is not symbolically equivalent to XLS\nquery:\n{query}",
        entry.id,
        ir_form.name(),
    );
}

#[test]
fn curated_manifest_and_validation_matrix_are_complete() {
    let entries = manifest_entries();
    assert!(
        !entries.is_empty(),
        "curated corpus manifest must not be empty"
    );
    let mut ids = BTreeSet::new();
    let mut fuzz_seeds = BTreeSet::new();
    for entry in &entries {
        assert!(ids.insert(entry.id), "duplicate corpus id: {}", entry.id);
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
        assert!(entry.fuzz_cases > 0, "{} fuzz case count", entry.id);
        assert!(
            fuzz_seeds.insert(entry.fuzz_seed),
            "duplicate fuzz seed: {:016x}",
            entry.fuzz_seed
        );
        assert!(!entry.source().is_empty(), "{} fixture", entry.id);
        assert!(
            !entry.curated_vectors().is_empty(),
            "{} curated vectors",
            entry.id
        );
    }

    let validations = validation_entries();
    assert_eq!(validations.len(), entries.len() * IrForm::ALL.len());
    let mut validation_keys = BTreeSet::new();
    for validation in validations {
        assert!(
            ids.contains(validation.id),
            "unknown validation id: {}",
            validation.id
        );
        assert!(
            validation_keys.insert((validation.id, validation.ir_form)),
            "duplicate validation row: {} {}",
            validation.id,
            validation.ir_form.name()
        );
        assert_eq!(validation.curated_vector_differential, "required");
        assert_eq!(validation.differential_fuzz, "required");
        assert_eq!(validation.symbolic_equivalence, "required");
    }
    for id in ids {
        for ir_form in IrForm::ALL {
            assert!(
                validation_keys.contains(&(id, ir_form)),
                "missing validation row: {} {}",
                id,
                ir_form.name()
            );
        }
    }
}

#[test]
fn curated_vector_differential_testing() {
    run_differential_validation(DifferentialMode::CuratedVector);
}

#[test]
fn differential_fuzz_testing() {
    run_differential_validation(DifferentialMode::Fuzz);
}

#[test]
fn symbolic_equivalence_checking() {
    for entry in manifest_entries() {
        let (unoptimized, optimized, function_name) = compile_entry(&entry);
        for (ir_form, package) in [
            (IrForm::Unoptimized, &unoptimized),
            (IrForm::Optimized, &optimized),
        ] {
            let function = package.get_function(&function_name).unwrap();
            let symbolic = evaluate_package(package, &function_name).unwrap_or_else(|error| {
                panic!(
                    "{} ({}): native symbolic evaluation failed: {error}",
                    entry.id,
                    ir_form.name()
                )
            });
            assert_symbolic_equivalence(&entry, ir_form, &function, &symbolic, &function_name);
        }
    }
}

#[test]
fn path_witness_replay_is_capability_gated() {
    for validation in validation_entries() {
        assert_eq!(
            validation.path_witness_replay,
            PATH_WITNESS_REPLAY_BLOCKER,
            "{} {} path-witness replay status",
            validation.id,
            validation.ir_form.name()
        );
    }
}
