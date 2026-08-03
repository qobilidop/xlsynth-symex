// SPDX-License-Identifier: Apache-2.0

//! End-to-end checks for canonical selection enumeration and witness replay.

mod common;

use std::collections::BTreeSet;

use xlsynth::{IrBits, IrPackage, IrValue};
use xlsynth_symex::{
    ConstraintComparison, ConstraintTerm, EnumerationCompleteness, EnumerationOptions,
    EnumerationResult, EvaluationInput, GuardedResult, IncompleteReason, InputConstraint,
    InputLeaf, SelectionOutcome, SymbolicValue, enumerate_package,
    enumerate_package_with_inputs_and_options, enumerate_package_with_options,
    enumerate_with_inputs, evaluate_package,
};

use common::run_z3;

fn parse(ir: &str, function_name: &str) -> (IrPackage, xlsynth::IrFunction) {
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let function = package.get_function(function_name).unwrap();
    (package, function)
}

fn flatten_ir_bits(value: &IrValue, output: &mut Vec<IrBits>) {
    if let Ok(elements) = value.get_elements() {
        for element in elements {
            flatten_ir_bits(&element, output);
        }
    } else {
        output.push(value.to_bits().unwrap());
    }
}

fn smt_bits(bits: &IrBits) -> String {
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

fn assert_witness_replays(function: &xlsynth::IrFunction, guarded: &GuardedResult) {
    let expected = function.interpret(&guarded.witness.inputs).unwrap();
    let mut expected_leaves = Vec::new();
    flatten_ir_bits(&expected, &mut expected_leaves);
    let mut symbolic_leaves = Vec::new();
    guarded.result.flatten_bits(&mut symbolic_leaves);
    assert_eq!(symbolic_leaves.len(), expected_leaves.len());

    let bindings = guarded
        .witness
        .symbolic_leaves
        .iter()
        .map(|(parameter, value)| {
            format!(
                "({} {})",
                parameter.name,
                smt_bits(&value.to_bits().unwrap())
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    let bind = |expression: &str| {
        if bindings.is_empty() {
            expression.to_owned()
        } else {
            format!("(let ({bindings}) {expression})")
        }
    };
    let mut equalities = symbolic_leaves
        .iter()
        .zip(&expected_leaves)
        .filter(|(symbolic, _)| symbolic.bit_count > 0)
        .map(|(symbolic, expected)| {
            format!("(= {} {})", bind(&symbolic.expression), smt_bits(expected))
        })
        .collect::<Vec<_>>();
    equalities.push(bind(guarded.guard.as_smtlib()));
    let query = format!(
        "(assert (not (and {})))\n(check-sat)\n",
        equalities.join(" ")
    );
    let stdout = run_z3(&query, "witness replay query");
    assert_eq!(
        stdout, "unsat",
        "witness result or guard disagrees with XLS\n{query}"
    );
}

fn outcomes(results: &[GuardedResult]) -> BTreeSet<SelectionOutcome> {
    results
        .iter()
        .flat_map(|guarded| guarded.trace.values().cloned())
        .collect()
}

fn assert_complete_partition(
    package: &IrPackage,
    function_name: &str,
    enumerated: &EnumerationResult,
) {
    assert_complete_partition_in_domain(package, function_name, enumerated, "true");
}

fn assert_complete_partition_in_domain(
    package: &IrPackage,
    function_name: &str,
    enumerated: &EnumerationResult,
    domain: &str,
) {
    assert_eq!(enumerated.completeness, EnumerationCompleteness::Complete);
    let merged = evaluate_package(package, function_name).unwrap();
    assert_eq!(enumerated.parameters, merged.parameters);
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
            assert_eq!(result_leaves.len(), merged_leaves.len());
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
        "{declarations}(assert (or (not (= {coverage} {domain})) {overlap} (not {equivalence})))\n(check-sat)\n"
    );
    let stdout = run_z3(&query, "selection-partition query");
    assert_eq!(
        stdout, "unsat",
        "selection partition is incomplete, overlapping, or differs from merged evaluation\n{query}"
    );
}

#[test]
fn nested_selects_are_complete_and_structurally_inactive() {
    let ir = r#"package test

top fn nested(x: bits[1] id=1, y: bits[1] id=2, a: bits[8] id=3, b: bits[8] id=4, c: bits[8] id=5) -> bits[8] {
  inner: bits[8] = sel(y, cases=[a, b], id=6)
  ret outer: bits[8] = sel(x, cases=[c, inner], id=7)
}
"#;
    let (package, function) = parse(ir, "nested");
    let result = enumerate_package(&package, "nested").unwrap();
    assert_complete_partition(&package, "nested", &result);
    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 3);
    let mut lengths = result
        .results
        .iter()
        .map(|guarded| guarded.trace.len())
        .collect::<Vec<_>>();
    lengths.sort_unstable();
    assert_eq!(lengths, [1, 2, 2]);
    for guarded in &result.results {
        assert_witness_replays(&function, guarded);
    }
}

#[test]
fn infeasible_correlated_nested_outcome_is_pruned() {
    let ir = r#"package test

top fn correlated(x: bits[1] id=1, a: bits[8] id=2, b: bits[8] id=3, c: bits[8] id=4) -> bits[8] {
  inner: bits[8] = sel(x, cases=[a, b], id=5)
  ret outer: bits[8] = sel(x, cases=[c, inner], id=6)
}
"#;
    let (package, function) = parse(ir, "correlated");
    let result = enumerate_package(&package, "correlated").unwrap();
    assert_complete_partition(&package, "correlated", &result);
    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 2);
    assert!(result.statistics.infeasible_candidates > 0);
    assert!(
        result
            .results
            .iter()
            .any(|guarded| guarded.trace.len() == 1)
    );
    assert!(
        result
            .results
            .iter()
            .any(|guarded| guarded.trace.len() == 2)
    );
    for guarded in &result.results {
        assert_witness_replays(&function, guarded);
    }
}

#[test]
fn priority_and_one_hot_outcomes_follow_v1_policy() {
    let priority_ir = r#"package test

top fn priority(s: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3, d: bits[8] id=4) -> bits[8] {
  ret result: bits[8] = priority_sel(s, cases=[a, b], default=d, id=5)
}
"#;
    let (priority_package, priority_function) = parse(priority_ir, "priority");
    let priority = enumerate_package(&priority_package, "priority").unwrap();
    assert_complete_partition(&priority_package, "priority", &priority);
    assert_eq!(priority.completeness, EnumerationCompleteness::Complete);
    assert_eq!(priority.results.len(), 3);
    assert_eq!(
        outcomes(&priority.results),
        BTreeSet::from([
            SelectionOutcome::Case(0),
            SelectionOutcome::Case(1),
            SelectionOutcome::Default,
        ])
    );
    for guarded in &priority.results {
        assert_witness_replays(&priority_function, guarded);
    }

    let one_hot_ir = r#"package test

top fn one_hot(s: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3) -> bits[8] {
  ret result: bits[8] = one_hot_sel(s, cases=[a, b], id=4)
}
"#;
    let (one_hot_package, one_hot_function) = parse(one_hot_ir, "one_hot");
    let one_hot = enumerate_package(&one_hot_package, "one_hot").unwrap();
    assert_complete_partition(&one_hot_package, "one_hot", &one_hot);
    assert_eq!(one_hot.completeness, EnumerationCompleteness::Complete);
    assert_eq!(one_hot.results.len(), 4);
    assert_eq!(
        outcomes(&one_hot.results),
        BTreeSet::from([
            SelectionOutcome::OneHotMask(vec![false, false]),
            SelectionOutcome::OneHotMask(vec![false, true]),
            SelectionOutcome::OneHotMask(vec![true, false]),
            SelectionOutcome::OneHotMask(vec![true, true]),
        ])
    );
    for guarded in &one_hot.results {
        assert_witness_replays(&one_hot_function, guarded);
    }
}

#[test]
fn concrete_selector_prunes_without_forking_and_records_its_outcome() {
    let ir = r#"package test

top fn select(s: bits[1] id=1, a: bits[8] id=2, b: bits[8] id=3) -> bits[8] {
  ret result: bits[8] = sel(s, cases=[a, b], id=4)
}
"#;
    let (_, function) = parse(ir, "select");
    let result = enumerate_with_inputs(
        &function,
        &[
            EvaluationInput::Concrete(IrValue::make_ubits(1, 1).unwrap()),
            EvaluationInput::Symbolic,
            EvaluationInput::Symbolic,
        ],
    )
    .unwrap();
    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.statistics.concrete_selections, 1);
    assert_eq!(result.statistics.symbolic_outcomes, 0);
    assert_eq!(result.statistics.solver_queries, 1);
    assert_eq!(result.results[0].trace.len(), 1);
    assert_eq!(
        outcomes(&result.results),
        BTreeSet::from([SelectionOutcome::Case(1)])
    );
    assert_eq!(
        result.results[0].result.as_bits().unwrap().expression,
        "symex_arg_2"
    );
    assert_witness_replays(&function, &result.results[0]);
}

#[test]
fn concrete_priority_and_one_hot_selections_record_without_forking() {
    let ir = r#"package test

top fn selections(priority: bits[2] id=1, mask: bits[2] id=2, a: bits[8] id=3, b: bits[8] id=4, d: bits[8] id=5) -> (bits[8], bits[8]) {
  p: bits[8] = priority_sel(priority, cases=[a, b], default=d, id=6)
  o: bits[8] = one_hot_sel(mask, cases=[a, b], id=7)
  ret result: (bits[8], bits[8]) = tuple(p, o, id=8)
}
"#;
    let (_, function) = parse(ir, "selections");
    let result = enumerate_with_inputs(
        &function,
        &[
            EvaluationInput::Concrete(IrValue::make_ubits(2, 2).unwrap()),
            EvaluationInput::Concrete(IrValue::make_ubits(2, 3).unwrap()),
            EvaluationInput::Symbolic,
            EvaluationInput::Symbolic,
            EvaluationInput::Symbolic,
        ],
    )
    .unwrap();
    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].trace.len(), 2);
    assert_eq!(result.statistics.concrete_selections, 2);
    assert_eq!(result.statistics.symbolic_outcomes, 0);
    assert_eq!(
        outcomes(&result.results),
        BTreeSet::from([
            SelectionOutcome::Case(1),
            SelectionOutcome::OneHotMask(vec![true, true]),
        ])
    );
    assert_witness_replays(&function, &result.results[0]);
}

#[test]
fn callsites_and_loop_iterations_have_distinct_selection_identities() {
    let invoke_ir = r#"package test

fn choose(s: bits[1] id=1, a: bits[8] id=2, b: bits[8] id=3) -> bits[8] {
  ret result: bits[8] = sel(s, cases=[a, b], id=4)
}

top fn invoke_twice(x: bits[1] id=5, y: bits[1] id=6, a: bits[8] id=7, b: bits[8] id=8) -> (bits[8], bits[8]) {
  left: bits[8] = invoke(x, a, b, to_apply=choose, id=9)
  right: bits[8] = invoke(y, a, b, to_apply=choose, id=10)
  ret result: (bits[8], bits[8]) = tuple(left, right, id=11)
}
"#;
    let (invoke_package, invoke_function) = parse(invoke_ir, "invoke_twice");
    let invoked = enumerate_package(&invoke_package, "invoke_twice").unwrap();
    assert_complete_partition(&invoke_package, "invoke_twice", &invoked);
    assert_eq!(invoked.completeness, EnumerationCompleteness::Complete);
    assert_eq!(invoked.results.len(), 4);
    for guarded in &invoked.results {
        assert_eq!(guarded.trace.len(), 2);
        let call_node_ids = guarded
            .trace
            .keys()
            .flat_map(|selection| selection.invocation.iter())
            .filter_map(|frame| match frame {
                xlsynth_symex::InvocationFrame::Invoke { node_id, .. } => Some(*node_id),
                xlsynth_symex::InvocationFrame::CountedFor { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(call_node_ids, BTreeSet::from([9, 10]));
        assert_witness_replays(&invoke_function, guarded);
    }

    let loop_ir = r#"package test

fn body(i: bits[2] id=1, carry: bits[8] id=2, s: bits[1] id=3) -> bits[8] {
  one: bits[8] = literal(value=1, id=4)
  added: bits[8] = add(carry, one, id=5)
  ret result: bits[8] = sel(s, cases=[carry, added], id=6)
}

top fn loop(s: bits[1] id=7, init: bits[8] id=8) -> bits[8] {
  ret result: bits[8] = counted_for(init, trip_count=2, stride=1, body=body, invariant_args=[s], id=9)
}
"#;
    let (loop_package, loop_function) = parse(loop_ir, "loop");
    let looped = enumerate_package(&loop_package, "loop").unwrap();
    assert_complete_partition(&loop_package, "loop", &looped);
    assert_eq!(looped.completeness, EnumerationCompleteness::Complete);
    assert_eq!(looped.results.len(), 2);
    for guarded in &looped.results {
        assert_eq!(guarded.trace.len(), 2);
        let iterations = guarded
            .trace
            .keys()
            .flat_map(|selection| selection.invocation.iter())
            .filter_map(|frame| match frame {
                xlsynth_symex::InvocationFrame::CountedFor { iteration, .. } => Some(*iteration),
                xlsynth_symex::InvocationFrame::Invoke { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(iterations, BTreeSet::from([0, 1]));
        assert_witness_replays(&loop_function, guarded);
    }
}

#[test]
fn configured_result_limit_is_never_reported_as_complete() {
    let ir = r#"package test

top fn one_hot(s: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3) -> bits[8] {
  ret result: bits[8] = one_hot_sel(s, cases=[a, b], id=4)
}
"#;
    let (package, _) = parse(ir, "one_hot");
    let result = enumerate_package_with_inputs_and_options(
        &package,
        "one_hot",
        &[
            EvaluationInput::Symbolic,
            EvaluationInput::Symbolic,
            EvaluationInput::Symbolic,
        ],
        &EnumerationOptions {
            max_results: Some(2),
            ..EnumerationOptions::default()
        },
    )
    .unwrap();
    assert_eq!(result.results.len(), 2);
    assert_eq!(
        result.completeness,
        EnumerationCompleteness::Incomplete(IncompleteReason::ResultLimit { limit: 2 })
    );
}

#[test]
fn caller_constraints_define_the_completed_input_domain() {
    let ir = r#"package test

top fn select(s: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3, d: bits[8] id=4) -> bits[8] {
  ret result: bits[8] = sel(s, cases=[a, b], default=d, id=5)
}
"#;
    let (package, function) = parse(ir, "select");
    let options = EnumerationOptions {
        max_results: None,
        constraints: vec![InputConstraint::Compare {
            operation: ConstraintComparison::UnsignedLessThan,
            lhs: ConstraintTerm::Input(InputLeaf::argument(0)),
            rhs: ConstraintTerm::Constant(IrValue::make_ubits(2, 2).unwrap()),
        }],
        ..EnumerationOptions::default()
    };
    let result = enumerate_package_with_options(&package, "select", &options).unwrap();
    assert_complete_partition_in_domain(
        &package,
        "select",
        &result,
        "(bvult symex_arg_0 (_ bv2 2))",
    );
    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 2);
    assert_eq!(
        outcomes(&result.results),
        BTreeSet::from([SelectionOutcome::Case(0), SelectionOutcome::Case(1)])
    );
    for guarded in &result.results {
        assert!(guarded.witness.inputs[0].to_u64().unwrap() < 2);
        assert_witness_replays(&function, guarded);
    }

    let invalid = EnumerationOptions {
        max_results: None,
        constraints: vec![InputConstraint::Compare {
            operation: ConstraintComparison::Equal,
            lhs: ConstraintTerm::Input(InputLeaf::argument(99)),
            rhs: ConstraintTerm::Constant(IrValue::make_ubits(2, 0).unwrap()),
        }],
        ..EnumerationOptions::default()
    };
    let error = enumerate_package_with_options(&package, "select", &invalid).unwrap_err();
    assert!(error.to_string().contains("non-symbolic input leaf"));
}

#[test]
fn constraints_address_structural_leaves_without_solver_names() {
    let ir = r#"package test

top fn select(input: (bits[1], bits[1]) id=1, a: bits[8] id=2, b: bits[8] id=3) -> bits[8] {
  selector: bits[1] = tuple_index(input, index=1, id=4)
  ret result: bits[8] = sel(selector, cases=[a, b], id=5)
}
"#;
    let (package, function) = parse(ir, "select");
    let selector_leaf = InputLeaf::argument(0).element(1);
    let options = EnumerationOptions {
        max_results: None,
        constraints: vec![InputConstraint::Compare {
            operation: ConstraintComparison::Equal,
            lhs: ConstraintTerm::Input(selector_leaf.clone()),
            rhs: ConstraintTerm::Constant(IrValue::make_ubits(1, 1).unwrap()),
        }],
        ..EnumerationOptions::default()
    };

    let result = enumerate_package_with_options(&package, "select", &options).unwrap();

    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.results.len(), 1);
    assert_eq!(
        outcomes(&result.results),
        BTreeSet::from([SelectionOutcome::Case(1)])
    );
    assert!(
        result
            .parameters
            .iter()
            .any(|parameter| parameter.input == selector_leaf)
    );
    assert_witness_replays(&function, &result.results[0]);
}

#[test]
fn structured_one_hot_results_are_deep_or_combined() {
    let ir = r#"package test

top fn structured(s: bits[2] id=1, a: bits[4] id=2, b: bits[4] id=3) -> (bits[4], bits[4]) {
  ta: (bits[4], bits[4]) = tuple(a, b, id=4)
  tb: (bits[4], bits[4]) = tuple(b, a, id=5)
  ret result: (bits[4], bits[4]) = one_hot_sel(s, cases=[ta, tb], id=6)
}
"#;
    let (package, function) = parse(ir, "structured");
    let result = enumerate_package(&package, "structured").unwrap();
    assert_eq!(result.results.len(), 4);
    assert!(matches!(result.results[0].result, SymbolicValue::Tuple(_)));
    for guarded in &result.results {
        assert_witness_replays(&function, guarded);
    }
}
