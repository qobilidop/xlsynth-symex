// SPDX-License-Identifier: Apache-2.0

//! Offline, provenance-pinned validation of curated upstream XLS examples.

mod common;

use std::collections::BTreeSet;
use std::path::Path;

use xlsynth::{
    DslxConvertOptions, IrFunction, IrPackage, IrValue, convert_dslx_to_ir, mangle_dslx_name,
    optimize_ir,
};
use xlsynth_pir::random_inputs::generate_flat_bitvector_argument_sets_from_seed;
use xlsynth_symex::{
    EnumerationCompleteness, EnumerationOptions, EnumerationResult, EvaluationInput, GuardedResult,
    SymexResult, enumerate_package, enumerate_package_with_inputs_and_options, evaluate_package,
    evaluate_package_with_inputs,
};

use common::run_z3;

const MANIFEST: &str = include_str!("corpus/curated/manifest.tsv");
const VALIDATION_MATRIX: &str = include_str!("corpus/curated/validation.tsv");
const UPSTREAM_REPOSITORY: &str = "https://github.com/xlsynth/xlsynth";
const UPSTREAM_REVISION: &str = "12bb182e4d842228878d6caf5489df5565c81aa0";
const XLS_REFERENCE_TRANSLATOR_BLOCKER: &str = "blocked:xls-reference-translator";

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
            "overflow_detect.x" => include_str!("corpus/curated/overflow_detect.x"),
            "lfsr.x" => include_str!("corpus/curated/lfsr.x"),
            "find_index.x" => include_str!("corpus/curated/find_index.x"),
            fixture => panic!("manifest references unknown curated fixture: {fixture}"),
        }
    }

    fn input_widths(&self) -> &'static [usize] {
        match self.id {
            "tiny_adder" => &[1, 1],
            "nested_sel" => &[8, 8, 8, 8, 8, 8],
            "riscv_decode_opcode" => &[32],
            "overflow_detect" => &[16, 16],
            "lfsr" => &[8],
            "find_index" => &[4, 4, 4, 4, 4],
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
            "overflow_detect" => vec![
                vec![(16, 0), (16, 0)],
                vec![(16, 15), (16, 16)],
                vec![(16, 255), (16, 1)],
                vec![(16, 16), (16, 16)],
                vec![(16, 65_535), (16, 65_535)],
            ],
            "lfsr" => vec![
                vec![(8, 0)],
                vec![(8, 1)],
                vec![(8, 37)],
                vec![(8, 155)],
                vec![(8, 237)],
                vec![(8, 255)],
            ],
            "find_index" => vec![
                vec![(4, 1), (4, 2), (4, 3), (4, 4), (4, 1)],
                vec![(4, 1), (4, 2), (4, 3), (4, 4), (4, 3)],
                vec![(4, 1), (4, 2), (4, 3), (4, 4), (4, 5)],
                vec![(4, 7), (4, 7), (4, 7), (4, 7), (4, 7)],
                vec![(4, 0), (4, 15), (4, 0), (4, 15), (4, 15)],
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

        generate_flat_bitvector_argument_sets_from_seed(
            self.input_widths(),
            self.fuzz_seed,
            self.fuzz_cases,
        )
        .into_iter()
        .map(|arguments| {
            arguments
                .into_iter()
                .map(|bits| (bits.get_bit_count(), bits.to_u64().unwrap()))
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
    selection_witness_replay: &'a str,
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
            selection_witness_replay: fields[5],
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

fn make_ir_args(entry: &ManifestEntry, sample: &BitsInput) -> Vec<IrValue> {
    if entry.id == "find_index" {
        let elements = sample[..4]
            .iter()
            .map(|(width, value)| IrValue::make_ubits(*width, *value).unwrap())
            .collect::<Vec<_>>();
        return vec![
            IrValue::make_array(&elements).unwrap(),
            IrValue::make_ubits(sample[4].0, sample[4].1).unwrap(),
        ];
    }
    sample
        .iter()
        .map(|(width, value)| IrValue::make_ubits(*width, *value).unwrap())
        .collect()
}

fn flatten_ir_bits(value: &IrValue, output: &mut Vec<(usize, u64)>) {
    if let Ok(elements) = value.get_elements() {
        for element in elements {
            flatten_ir_bits(&element, output);
        }
    } else {
        output.push((value.bit_count().unwrap(), value.to_u64().unwrap()));
    }
}

fn smt_ir_value_bits(value: &IrValue) -> String {
    let bits = value.to_bits().unwrap();
    let mut result = String::from("#b");
    for index in (0..bits.get_bit_count()).rev() {
        result.push(if bits.get_bit(index).unwrap() {
            '1'
        } else {
            '0'
        });
    }
    result
}

fn assert_witness_replays(
    entry: &ManifestEntry,
    ir_form: IrForm,
    package: &IrPackage,
    function_name: &str,
    function: &IrFunction,
    result_index: usize,
    guarded: &GuardedResult,
) {
    let expected = function
        .interpret(&guarded.witness.inputs)
        .unwrap_or_else(|error| {
            panic!(
                "{} ({} result {result_index}): XLS witness replay failed: {error}",
                entry.id,
                ir_form.name()
            )
        });
    let mut expected_leaves = Vec::new();
    flatten_ir_bits(&expected, &mut expected_leaves);
    let mut result_leaves = Vec::new();
    guarded.result.flatten_bits(&mut result_leaves);
    assert_eq!(result_leaves.len(), expected_leaves.len());
    let bindings = guarded
        .witness
        .symbolic_leaves
        .iter()
        .map(|(parameter, value)| format!("({} {})", parameter.name, smt_ir_value_bits(value)))
        .collect::<Vec<_>>()
        .join(" ");
    let bind = |expression: &str| {
        if bindings.is_empty() {
            expression.to_owned()
        } else {
            format!("(let ({bindings}) {expression})")
        }
    };
    let mut claims = result_leaves
        .iter()
        .zip(&expected_leaves)
        .filter(|(actual, _)| actual.bit_count > 0)
        .map(|(actual, (width, expected))| {
            format!("(= {} (_ bv{expected} {width}))", bind(&actual.expression))
        })
        .collect::<Vec<_>>();
    claims.push(bind(guarded.guard.as_smtlib()));
    let query = format!("(assert (not (and {})))\n(check-sat)\n", claims.join(" "));
    let context = format!(
        "{} ({} result {result_index}) witness",
        entry.id,
        ir_form.name()
    );
    let stdout = run_z3(&query, &context);
    assert_eq!(
        stdout,
        "unsat",
        "{} ({} result {result_index}) witness does not satisfy its guard/result\n{query}",
        entry.id,
        ir_form.name()
    );

    let concrete_inputs = guarded
        .witness
        .inputs
        .iter()
        .cloned()
        .map(EvaluationInput::Concrete)
        .collect::<Vec<_>>();
    let concrete_enumeration = enumerate_package_with_inputs_and_options(
        package,
        function_name,
        &concrete_inputs,
        &EnumerationOptions::default(),
    )
    .unwrap_or_else(|error| {
        panic!(
            "{} ({} result {result_index}): concrete trace replay failed: {error}",
            entry.id,
            ir_form.name()
        )
    });
    assert_eq!(
        concrete_enumeration.completeness,
        EnumerationCompleteness::Complete,
        "{} ({} result {result_index}): concrete trace replay incomplete",
        entry.id,
        ir_form.name()
    );
    assert_eq!(
        concrete_enumeration.results.len(),
        1,
        "{} ({} result {result_index}): concrete inputs must determine one guarded result",
        entry.id,
        ir_form.name()
    );
    assert_eq!(
        concrete_enumeration.results[0].trace,
        guarded.trace,
        "{} ({} result {result_index}): concrete and symbolic traces differ",
        entry.id,
        ir_form.name()
    );
}

fn assert_complete_partition(
    entry: &ManifestEntry,
    ir_form: IrForm,
    partition: &str,
    merged: &SymexResult,
    enumerated: &EnumerationResult,
) {
    assert_eq!(
        enumerated.completeness,
        EnumerationCompleteness::Complete,
        "{} ({} {partition}): enumeration incomplete",
        entry.id,
        ir_form.name()
    );
    assert_eq!(
        enumerated.parameters,
        merged.parameters,
        "{} ({} {partition}): merged/enumerated parameters differ",
        entry.id,
        ir_form.name()
    );
    let unique_traces = enumerated
        .results
        .iter()
        .map(|guarded| guarded.trace.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        unique_traces.len(),
        enumerated.results.len(),
        "{} ({} {partition}): duplicate canonical traces",
        entry.id,
        ir_form.name()
    );

    let declarations = enumerated
        .parameters
        .iter()
        .map(|parameter| {
            format!(
                "(declare-const {} (_ BitVec {}))\n",
                parameter.name, parameter.bit_count
            )
        })
        .collect::<String>();
    let guards = enumerated
        .results
        .iter()
        .map(|guarded| guarded.guard.as_smtlib())
        .collect::<Vec<_>>();
    let coverage = match guards.as_slice() {
        [] => "false".to_owned(),
        [only] => (*only).to_owned(),
        _ => format!("(or {})", guards.join(" ")),
    };
    let overlaps = guards
        .iter()
        .enumerate()
        .flat_map(|(lhs_index, lhs)| {
            guards
                .iter()
                .skip(lhs_index + 1)
                .map(move |rhs| format!("(and {lhs} {rhs})"))
        })
        .collect::<Vec<_>>();
    let overlap = match overlaps.as_slice() {
        [] => "false".to_owned(),
        [only] => only.clone(),
        _ => format!("(or {})", overlaps.join(" ")),
    };
    let mut merged_leaves = Vec::new();
    merged.result.flatten_bits(&mut merged_leaves);
    let implications = enumerated
        .results
        .iter()
        .map(|guarded| {
            let mut result_leaves = Vec::new();
            guarded.result.flatten_bits(&mut result_leaves);
            assert_eq!(
                result_leaves.len(),
                merged_leaves.len(),
                "{} ({} {partition}): result shapes differ",
                entry.id,
                ir_form.name()
            );
            let equalities = result_leaves
                .iter()
                .zip(&merged_leaves)
                .filter(|(result_leaf, _)| result_leaf.bit_count > 0)
                .map(|(result_leaf, merged)| {
                    format!("(= {} {})", result_leaf.expression, merged.expression)
                })
                .collect::<Vec<_>>();
            let equality = match equalities.as_slice() {
                [] => "true".to_owned(),
                [only] => only.clone(),
                _ => format!("(and {})", equalities.join(" ")),
            };
            format!("(=> {} {equality})", guarded.guard.as_smtlib())
        })
        .collect::<Vec<_>>();
    let equivalence = match implications.as_slice() {
        [] => "false".to_owned(),
        [only] => only.clone(),
        _ => format!("(and {})", implications.join(" ")),
    };
    let query = format!(
        "{declarations}(assert (or (not {coverage}) {overlap} (not {equivalence})))\n(check-sat)\n"
    );
    let context = format!("{} ({} {partition}) partition", entry.id, ir_form.name());
    let stdout = run_z3(&query, &context);
    assert_eq!(
        stdout,
        "unsat",
        "{} ({} {partition}): incomplete or overlapping domain, or piecewise mismatch\n{query}",
        entry.id,
        ir_form.name()
    );
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
    let parameter_declarations = symbolic
        .parameters
        .iter()
        .map(|parameter| format!("({} (_ BitVec {}))", parameter.name, parameter.bit_count))
        .collect::<Vec<_>>()
        .join(" ");
    let mut result_leaves = Vec::new();
    symbolic.result.flatten_bits(&mut result_leaves);
    let candidate = result_leaves
        .iter()
        .enumerate()
        .filter(|(_, bits)| bits.bit_count > 0)
        .map(|(index, bits)| {
            format!(
                "(define-fun xlsynth_symex_apply_{index} ({parameter_declarations}) (_ BitVec {}) {})\n",
                bits.bit_count, bits.expression
            )
        })
        .collect::<String>();
    let mut mismatches = Vec::with_capacity(samples.len());
    for (case, sample) in samples.iter().enumerate() {
        let args = make_ir_args(entry, sample);
        let expected = function.interpret(&args).unwrap_or_else(|error| {
            panic!(
                "{} ({}, {} case {case}): XLS interpreter failed for {sample:?}: {error}",
                entry.id,
                ir_form.name(),
                mode.name()
            )
        });
        let mut expected_leaves = Vec::new();
        flatten_ir_bits(&expected, &mut expected_leaves);
        assert_eq!(result_leaves.len(), expected_leaves.len());
        assert_eq!(symbolic.parameters.len(), sample.len());
        let arguments = symbolic
            .parameters
            .iter()
            .zip(sample)
            .map(|(parameter, (width, value))| {
                assert_eq!(parameter.bit_count, *width);
                format!(" (_ bv{value} {width})")
            })
            .collect::<String>();
        let leaf_mismatches = expected_leaves
            .iter()
            .enumerate()
            .filter(|(_, (width, _))| *width > 0)
            .map(|(index, (width, expected))| {
                format!(
                    "(not (= (xlsynth_symex_apply_{index}{arguments}) (_ bv{expected} {width})))"
                )
            })
            .collect::<Vec<_>>();
        mismatches.push(if leaf_mismatches.len() == 1 {
            leaf_mismatches[0].clone()
        } else {
            format!("(or {})", leaf_mismatches.join(" "))
        });
    }

    let query = format!(
        "{candidate}(assert (or\n  {}))\n(check-sat)\n",
        mismatches.join("\n  ")
    );
    let context = format!(
        "{} ({}, {}) differential over {} samples",
        entry.id,
        ir_form.name(),
        mode.name(),
        samples.len()
    );
    let stdout = run_z3(&query, &context);
    assert_eq!(
        stdout,
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
    let comparison = if entry.id == "find_index" {
        find_index_equivalence_comparison(symbolic, function_name)
    } else {
        let arguments = symbolic
            .parameters
            .iter()
            .map(|parameter| format!(" {}", parameter.name))
            .collect::<String>();
        format!("(= xlsynth_symex_result (select {function_name}{arguments}))")
    };
    let query = format!(
        "{}\n{reference}\n(assert (not {comparison}))\n(check-sat)\n",
        symbolic.result_smtlib
    );
    let context = format!("{} ({}) symbolic equivalence", entry.id, ir_form.name());
    let stdout = run_z3(&query, &context);
    assert_eq!(
        stdout,
        "unsat",
        "{} ({}) native result is not symbolically equivalent to XLS\nquery:\n{query}",
        entry.id,
        ir_form.name(),
    );
}

fn find_index_equivalence_comparison(symbolic: &SymexResult, function_name: &str) -> String {
    assert_eq!(symbolic.parameters.len(), 5);
    let mut array = "((as const (Array (_ BitVec 3) (_ BitVec 4))) (_ bv0 4))".to_owned();
    for (index, parameter) in symbolic.parameters[..4].iter().enumerate() {
        array = format!("(store {array} (_ bv{index} 3) {})", parameter.name);
    }
    let reference_result = format!(
        "(select {function_name} {array} {})",
        symbolic.parameters[4].name
    );
    let mut native_leaves = Vec::new();
    symbolic.result.flatten_bits(&mut native_leaves);
    assert_eq!(native_leaves.len(), 2);
    format!(
        "(and (= {} (|(bits[1], bits[2])_0| {reference_result})) (= {} (|(bits[1], bits[2])_1| {reference_result})))",
        native_leaves[0].expression, native_leaves[1].expression
    )
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
            entry.argument_partition, "all-symbolic,each-argument-concrete",
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
        let entry = entries
            .iter()
            .find(|entry| entry.id == validation.id)
            .unwrap();
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
        assert_eq!(
            validation.curated_vector_differential,
            format!("pass:{}", entry.curated_vectors().len())
        );
        assert_eq!(
            validation.differential_fuzz,
            format!("pass:{}", entry.fuzz_cases)
        );
        assert!(
            matches!(
                validation.symbolic_equivalence,
                "unsat" | XLS_REFERENCE_TRANSLATOR_BLOCKER
            ),
            "{} {} invalid symbolic equivalence status: {}",
            validation.id,
            validation.ir_form.name(),
            validation.symbolic_equivalence
        );
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
            let validation = validation_entries()
                .into_iter()
                .find(|validation| validation.id == entry.id && validation.ir_form == ir_form)
                .unwrap();
            if matches!(
                validation.symbolic_equivalence,
                XLS_REFERENCE_TRANSLATOR_BLOCKER
            ) {
                continue;
            }
            assert_eq!(validation.symbolic_equivalence, "unsat");
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
fn selection_witness_replay() {
    let mut matrix_mismatches = Vec::new();
    for entry in manifest_entries() {
        let (unoptimized, optimized, function_name) = compile_entry(&entry);
        for (ir_form, package) in [
            (IrForm::Unoptimized, &unoptimized),
            (IrForm::Optimized, &optimized),
        ] {
            let validation = validation_entries()
                .into_iter()
                .find(|validation| validation.id == entry.id && validation.ir_form == ir_form)
                .unwrap();
            let expected_results = validation
                .selection_witness_replay
                .strip_prefix("pass:")
                .and_then(|count| count.parse::<usize>().ok());
            let enumerated = enumerate_package(package, &function_name).unwrap_or_else(|error| {
                panic!(
                    "{} ({}): selection enumeration failed: {error}",
                    entry.id,
                    ir_form.name()
                )
            });
            let merged = evaluate_package(package, &function_name).unwrap_or_else(|error| {
                panic!(
                    "{} ({}): merged evaluation failed during partition proof: {error}",
                    entry.id,
                    ir_form.name()
                )
            });
            assert_complete_partition(&entry, ir_form, "all-symbolic", &merged, &enumerated);
            if expected_results != Some(enumerated.results.len()) {
                matrix_mismatches.push(format!(
                    "{}\t{}\tpass:{} (recorded {})",
                    entry.id,
                    ir_form.name(),
                    enumerated.results.len(),
                    validation.selection_witness_replay
                ));
            }
            let function = package.get_function(&function_name).unwrap();
            for (result_index, guarded) in enumerated.results.iter().enumerate() {
                assert_witness_replays(
                    &entry,
                    ir_form,
                    package,
                    &function_name,
                    &function,
                    result_index,
                    guarded,
                );
            }
        }
    }
    assert!(
        matrix_mismatches.is_empty(),
        "selection-witness validation matrix is stale:\n{}",
        matrix_mismatches.join("\n")
    );
}

#[test]
fn mixed_argument_partition_witness_replay() {
    for entry in manifest_entries() {
        let concrete_args = make_ir_args(&entry, &entry.curated_vectors()[0]);
        let (unoptimized, optimized, function_name) = compile_entry(&entry);
        for (ir_form, package) in [
            (IrForm::Unoptimized, &unoptimized),
            (IrForm::Optimized, &optimized),
        ] {
            let function = package.get_function(&function_name).unwrap();
            for concrete_index in 0..concrete_args.len() {
                let inputs = concrete_args
                    .iter()
                    .enumerate()
                    .map(|(index, value)| {
                        if index == concrete_index {
                            EvaluationInput::Concrete(value.clone())
                        } else {
                            EvaluationInput::Symbolic
                        }
                    })
                    .collect::<Vec<_>>();
                let enumerated = enumerate_package_with_inputs_and_options(
                    package,
                    &function_name,
                    &inputs,
                    &EnumerationOptions::default(),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "{} ({} partition concrete:{concrete_index}): enumeration failed: {error}",
                        entry.id,
                        ir_form.name()
                    )
                });
                let merged = evaluate_package_with_inputs(package, &function_name, &inputs)
                    .unwrap_or_else(|error| {
                        panic!(
                            "{} ({} partition concrete:{concrete_index}): merged evaluation failed: {error}",
                            entry.id,
                            ir_form.name()
                        )
                    });
                let partition = format!("concrete:{concrete_index}");
                assert_complete_partition(&entry, ir_form, &partition, &merged, &enumerated);
                for (result_index, guarded) in enumerated.results.iter().enumerate() {
                    assert_witness_replays(
                        &entry,
                        ir_form,
                        package,
                        &function_name,
                        &function,
                        result_index,
                        guarded,
                    );
                }
            }
        }
    }
}
