// SPDX-License-Identifier: Apache-2.0

//! Independent bounded trace-set and enumeration-mutation release checks.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use xlsynth::IrPackage;
use xlsynth_symex::{
    EnumerationCompleteness, EnumerationResult, InvocationFrame, SelectionId, SelectionOutcome,
    enumerate_package, evaluate_package,
};

use common::run_z3;

#[derive(Clone, Debug)]
enum SelectionTree {
    Leaf(u8),
    Sel {
        node_id: usize,
        selector: usize,
        cases: [Box<SelectionTree>; 2],
        default: Box<SelectionTree>,
    },
}
struct DeterministicRng(u64);

impl DeterministicRng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn generate_tree(rng: &mut DeterministicRng, depth: usize) -> SelectionTree {
    if depth == 0 {
        return SelectionTree::Leaf(rng.next() as u8);
    }
    SelectionTree::Sel {
        node_id: 0,
        selector: rng.next() as usize % 3,
        cases: [
            Box::new(generate_tree(rng, depth - 1)),
            Box::new(generate_tree(rng, depth - 1)),
        ],
        default: Box::new(generate_tree(rng, depth - 1)),
    }
}

fn emit_tree(tree: &mut SelectionTree, next_id: &mut usize, lines: &mut String) -> String {
    match tree {
        SelectionTree::Leaf(value) => {
            let id = *next_id;
            *next_id += 1;
            lines.push_str(&format!(
                "  n{id}: bits[8] = literal(value={value}, id={id})\n"
            ));
            format!("n{id}")
        }
        SelectionTree::Sel {
            node_id,
            selector,
            cases,
            default,
        } => {
            let case0 = emit_tree(&mut cases[0], next_id, lines);
            let case1 = emit_tree(&mut cases[1], next_id, lines);
            let default = emit_tree(default, next_id, lines);
            let id = *next_id;
            *next_id += 1;
            *node_id = id;
            lines.push_str(&format!(
                "  n{id}: bits[8] = sel(s{selector}, cases=[{case0}, {case1}], default={default}, id={id})\n"
            ));
            format!("n{id}")
        }
    }
}

fn render_generated_function(seed: u64) -> (String, SelectionTree) {
    let mut rng = DeterministicRng(seed);
    let mut tree = generate_tree(&mut rng, 3);
    let mut next_id = 4;
    let mut lines = String::new();
    let root = emit_tree(&mut tree, &mut next_id, &mut lines);
    let ir = format!(
        "package generated\n\ntop fn generated(s0: bits[2] id=1, s1: bits[2] id=2, s2: bits[2] id=3) -> bits[8] {{\n{lines}  ret result: bits[8] = identity({root}, id={next_id})\n}}\n"
    );
    (ir, tree)
}

fn concrete_tree_trace(
    tree: &SelectionTree,
    selectors: &[usize; 3],
    trace: &mut BTreeMap<usize, SelectionOutcome>,
) {
    let SelectionTree::Sel {
        node_id,
        selector,
        cases,
        default,
    } = tree
    else {
        return;
    };
    let value = selectors[*selector];
    let (outcome, selected) = match value {
        0 | 1 => (SelectionOutcome::Case(value), cases[value].as_ref()),
        _ => (SelectionOutcome::Default, default.as_ref()),
    };
    assert!(trace.insert(*node_id, outcome).is_none());
    concrete_tree_trace(selected, selectors, trace);
}

fn static_trace(guarded: &xlsynth_symex::GuardedResult) -> BTreeMap<usize, SelectionOutcome> {
    guarded
        .trace
        .iter()
        .map(|(selection, outcome)| {
            assert!(selection.invocation.is_empty());
            (selection.node_id, outcome.clone())
        })
        .collect()
}

#[test]
fn generated_selection_trees_match_exhaustive_concrete_trace_sets() {
    for seed in [
        0x5eed_1000_0000_0001,
        0x5eed_1000_0000_0002,
        0x5eed_1000_0000_0003,
        0x5eed_1000_0000_0004,
    ] {
        let (ir, tree) = render_generated_function(seed);
        let package = IrPackage::parse_ir(&ir, None)
            .unwrap_or_else(|error| panic!("seed {seed:016x}: {error}\n{ir}"));
        let enumerated = enumerate_package(&package, "generated").unwrap();
        assert_eq!(
            enumerated.completeness,
            EnumerationCompleteness::Complete,
            "seed {seed:016x}"
        );
        let actual = enumerated
            .results
            .iter()
            .map(static_trace)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual.len(), enumerated.results.len());
        let mut expected = BTreeSet::new();
        for packed in 0..64 {
            let selectors = [packed & 3, (packed >> 2) & 3, (packed >> 4) & 3];
            let mut trace = BTreeMap::new();
            concrete_tree_trace(&tree, &selectors, &mut trace);
            expected.insert(trace);
        }
        assert_eq!(actual, expected, "seed {seed:016x}\n{ir}");
    }
}

#[test]
fn priority_and_one_hot_cross_product_matches_exhaustive_selectors() {
    let ir = r#"package generated

top fn generated(priority: bits[2] id=1, mask: bits[2] id=2, a: bits[8] id=3, b: bits[8] id=4, d: bits[8] id=5) -> (bits[8], bits[8]) {
  p: bits[8] = priority_sel(priority, cases=[a, b], default=d, id=6)
  o: bits[8] = one_hot_sel(mask, cases=[a, b], id=7)
  ret result: (bits[8], bits[8]) = tuple(p, o, id=8)
}
"#;
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let enumerated = enumerate_package(&package, "generated").unwrap();
    assert_eq!(enumerated.completeness, EnumerationCompleteness::Complete);
    let actual = enumerated
        .results
        .iter()
        .map(static_trace)
        .collect::<BTreeSet<_>>();
    let mut expected = BTreeSet::new();
    for priority in 0..4 {
        for mask in 0..4 {
            let priority_outcome = if priority & 1 != 0 {
                SelectionOutcome::Case(0)
            } else if priority & 2 != 0 {
                SelectionOutcome::Case(1)
            } else {
                SelectionOutcome::Default
            };
            expected.insert(BTreeMap::from([
                (6, priority_outcome),
                (
                    7,
                    SelectionOutcome::OneHotMask(vec![mask & 1 != 0, mask & 2 != 0]),
                ),
            ]));
        }
    }
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 12);
}

#[derive(Clone)]
struct Candidate {
    guard: String,
    result: String,
    trace: BTreeMap<SelectionId, SelectionOutcome>,
}

fn candidates(enumerated: &EnumerationResult) -> Vec<Candidate> {
    enumerated
        .results
        .iter()
        .map(|guarded| Candidate {
            guard: guarded.guard.as_smtlib().to_owned(),
            result: guarded.result.as_bits().unwrap().expression.clone(),
            trace: guarded.trace.clone(),
        })
        .collect()
}

fn verifier_accepts(
    enumerated: &EnumerationResult,
    merged: &str,
    expected_traces: &BTreeSet<BTreeMap<SelectionId, SelectionOutcome>>,
    candidates: &[Candidate],
) -> bool {
    let traces = candidates
        .iter()
        .map(|candidate| candidate.trace.clone())
        .collect::<BTreeSet<_>>();
    if traces.len() != candidates.len() || &traces != expected_traces {
        return false;
    }
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
    let coverage = format!(
        "(or {})",
        candidates
            .iter()
            .map(|candidate| candidate.guard.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let overlaps = candidates
        .iter()
        .enumerate()
        .flat_map(|(lhs_index, lhs)| {
            candidates
                .iter()
                .skip(lhs_index + 1)
                .map(move |rhs| format!("(and {} {})", lhs.guard, rhs.guard))
        })
        .collect::<Vec<_>>();
    let overlap = match overlaps.as_slice() {
        [] => "false".to_owned(),
        [only] => only.clone(),
        _ => format!("(or {})", overlaps.join(" ")),
    };
    let equivalence = format!(
        "(and {})",
        candidates
            .iter()
            .map(|candidate| format!("(=> {} (= {} {merged}))", candidate.guard, candidate.result))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let query = format!(
        "{declarations}(assert (or (not {coverage}) {overlap} (not {equivalence})))\n(check-sat)\n"
    );
    run_z3(&query, "enumeration-completeness query") == "unsat"
}

#[test]
fn release_verifier_rejects_enumeration_mutations() {
    let ir = r#"package mutations

top fn choose(s: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3, d: bits[8] id=4) -> bits[8] {
  ret result: bits[8] = sel(s, cases=[a, b], default=d, id=5)
}
"#;
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let enumerated = enumerate_package(&package, "choose").unwrap();
    let merged = evaluate_package(&package, "choose")
        .unwrap()
        .result
        .as_bits()
        .unwrap()
        .expression
        .clone();
    let baseline = candidates(&enumerated);
    let expected_traces = baseline
        .iter()
        .map(|candidate| candidate.trace.clone())
        .collect::<BTreeSet<_>>();
    assert!(verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &baseline
    ));

    let mut omitted = baseline.clone();
    omitted.pop();
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &omitted
    ));

    let mut duplicated = baseline.clone();
    duplicated.push(baseline[0].clone());
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &duplicated
    ));

    let mut relabeled = baseline.clone();
    *relabeled[0].trace.values_mut().next().unwrap() = SelectionOutcome::Case(1);
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &relabeled
    ));

    let mut weakened = baseline.clone();
    weakened[0].guard = "true".to_owned();
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &weakened
    ));

    let mut strengthened = baseline.clone();
    strengthened[0].guard = format!("(and {} (= symex_arg_1 #b00000000))", strengthened[0].guard);
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &strengthened
    ));

    let mut incorrectly_active = baseline.clone();
    incorrectly_active[0].trace.insert(
        SelectionId {
            function: "choose".to_owned(),
            node_id: 99,
            invocation: vec![InvocationFrame::Invoke {
                caller: "inactive".to_owned(),
                node_id: 98,
            }],
        },
        SelectionOutcome::Case(0),
    );
    assert!(!verifier_accepts(
        &enumerated,
        &merged,
        &expected_traces,
        &incorrectly_active
    ));
}
