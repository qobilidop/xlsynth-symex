// SPDX-License-Identifier: Apache-2.0

//! End-to-end checks for canonical path enumeration and witness replay.

use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

use xlsynth::{IrBits, IrPackage, IrValue};
use xlsynth_symex::{
    ChoiceOutcome, ConstraintComparison, ConstraintTerm, EnumerationCompleteness,
    EnumerationOptions, EnumerationResult, EvaluationInput, IncompleteReason, InputConstraint,
    PathResult, SymbolicValue, enumerate_package, enumerate_package_with_inputs_and_options,
    enumerate_package_with_options, enumerate_with_inputs, evaluate_package,
};

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

fn assert_witness_replays(function: &xlsynth::IrFunction, path: &PathResult) {
    let expected = function.interpret(&path.witness.inputs).unwrap();
    let mut expected_leaves = Vec::new();
    flatten_ir_bits(&expected, &mut expected_leaves);
    let mut symbolic_leaves = Vec::new();
    path.result.flatten_bits(&mut symbolic_leaves);
    assert_eq!(symbolic_leaves.len(), expected_leaves.len());

    let bindings = path
        .witness
        .symbolic_leaves
        .iter()
        .map(|(name, value)| format!("({name} {})", smt_bits(&value.to_bits().unwrap())))
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
    equalities.push(bind(path.condition.as_smtlib()));
    let query = format!(
        "(assert (not (and {})))\n(check-sat)\n",
        equalities.join(" ")
    );
    let mut child = Command::new("z3")
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(query.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "witness result or condition disagrees with XLS\n{query}"
    );
}

fn outcomes(paths: &[PathResult]) -> BTreeSet<ChoiceOutcome> {
    paths
        .iter()
        .flat_map(|path| path.trace.values().cloned())
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
    let coverage = match enumerated.paths.as_slice() {
        [] => "false".to_owned(),
        [only] => only.condition.as_smtlib().to_owned(),
        _ => format!(
            "(or {})",
            enumerated
                .paths
                .iter()
                .map(|path| path.condition.as_smtlib())
                .collect::<Vec<_>>()
                .join(" ")
        ),
    };
    let mut merged_leaves = Vec::new();
    merged.result.flatten_bits(&mut merged_leaves);
    let implications = enumerated
        .paths
        .iter()
        .map(|path| {
            let mut path_leaves = Vec::new();
            path.result.flatten_bits(&mut path_leaves);
            assert_eq!(path_leaves.len(), merged_leaves.len());
            let equalities = path_leaves
                .iter()
                .zip(&merged_leaves)
                .filter(|(path, _)| path.bit_count > 0)
                .map(|(path, merged)| format!("(= {} {})", path.expression, merged.expression))
                .collect::<Vec<_>>();
            let equality = match equalities.as_slice() {
                [] => "true".to_owned(),
                [only] => only.clone(),
                _ => format!("(and {})", equalities.join(" ")),
            };
            format!("(=> {} {equality})", path.condition.as_smtlib())
        })
        .collect::<Vec<_>>();
    let equivalence = match implications.as_slice() {
        [] => "false".to_owned(),
        [only] => only.clone(),
        _ => format!("(and {})", implications.join(" ")),
    };
    let query = format!(
        "{declarations}(assert (or (not (= {coverage} {domain})) (not {equivalence})))\n(check-sat)\n"
    );
    let mut child = Command::new("z3")
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(query.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "path domain is incomplete or piecewise result differs from merged evaluation\n{query}"
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
    assert_eq!(result.paths.len(), 3);
    let mut lengths = result
        .paths
        .iter()
        .map(|path| path.trace.len())
        .collect::<Vec<_>>();
    lengths.sort_unstable();
    assert_eq!(lengths, [1, 2, 2]);
    for path in &result.paths {
        assert_witness_replays(&function, path);
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
    assert_eq!(result.paths.len(), 2);
    assert!(result.statistics.infeasible_candidates > 0);
    assert!(result.paths.iter().any(|path| path.trace.len() == 1));
    assert!(result.paths.iter().any(|path| path.trace.len() == 2));
    for path in &result.paths {
        assert_witness_replays(&function, path);
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
    assert_eq!(priority.paths.len(), 3);
    assert_eq!(
        outcomes(&priority.paths),
        BTreeSet::from([
            ChoiceOutcome::Case(0),
            ChoiceOutcome::Case(1),
            ChoiceOutcome::Default,
        ])
    );
    for path in &priority.paths {
        assert_witness_replays(&priority_function, path);
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
    assert_eq!(one_hot.paths.len(), 4);
    assert_eq!(
        outcomes(&one_hot.paths),
        BTreeSet::from([
            ChoiceOutcome::OneHotMask(vec![false, false]),
            ChoiceOutcome::OneHotMask(vec![false, true]),
            ChoiceOutcome::OneHotMask(vec![true, false]),
            ChoiceOutcome::OneHotMask(vec![true, true]),
        ])
    );
    for path in &one_hot.paths {
        assert_witness_replays(&one_hot_function, path);
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
    assert_eq!(result.paths.len(), 1);
    assert_eq!(result.statistics.concrete_choices, 1);
    assert_eq!(result.statistics.symbolic_outcomes, 0);
    assert_eq!(result.statistics.solver_queries, 1);
    assert_eq!(result.paths[0].trace.len(), 1);
    assert_eq!(
        outcomes(&result.paths),
        BTreeSet::from([ChoiceOutcome::Case(1)])
    );
    assert_eq!(
        result.paths[0].result.as_bits().unwrap().expression,
        "symex_arg_2"
    );
    assert_witness_replays(&function, &result.paths[0]);
}

#[test]
fn concrete_priority_and_one_hot_choices_record_without_forking() {
    let ir = r#"package test

top fn choices(priority: bits[2] id=1, mask: bits[2] id=2, a: bits[8] id=3, b: bits[8] id=4, d: bits[8] id=5) -> (bits[8], bits[8]) {
  p: bits[8] = priority_sel(priority, cases=[a, b], default=d, id=6)
  o: bits[8] = one_hot_sel(mask, cases=[a, b], id=7)
  ret result: (bits[8], bits[8]) = tuple(p, o, id=8)
}
"#;
    let (_, function) = parse(ir, "choices");
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
    assert_eq!(result.paths.len(), 1);
    assert_eq!(result.paths[0].trace.len(), 2);
    assert_eq!(result.statistics.concrete_choices, 2);
    assert_eq!(result.statistics.symbolic_outcomes, 0);
    assert_eq!(
        outcomes(&result.paths),
        BTreeSet::from([
            ChoiceOutcome::Case(1),
            ChoiceOutcome::OneHotMask(vec![true, true]),
        ])
    );
    assert_witness_replays(&function, &result.paths[0]);
}

#[test]
fn callsites_and_loop_iterations_have_distinct_choice_identities() {
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
    assert_eq!(invoked.paths.len(), 4);
    for path in &invoked.paths {
        assert_eq!(path.trace.len(), 2);
        let call_node_ids = path
            .trace
            .keys()
            .flat_map(|choice| choice.invocation.iter())
            .filter_map(|frame| match frame {
                xlsynth_symex::InvocationFrame::Invoke { node_id, .. } => Some(*node_id),
                xlsynth_symex::InvocationFrame::CountedFor { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(call_node_ids, BTreeSet::from([9, 10]));
        assert_witness_replays(&invoke_function, path);
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
    assert_eq!(looped.paths.len(), 2);
    for path in &looped.paths {
        assert_eq!(path.trace.len(), 2);
        let iterations = path
            .trace
            .keys()
            .flat_map(|choice| choice.invocation.iter())
            .filter_map(|frame| match frame {
                xlsynth_symex::InvocationFrame::CountedFor { iteration, .. } => Some(*iteration),
                xlsynth_symex::InvocationFrame::Invoke { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(iterations, BTreeSet::from([0, 1]));
        assert_witness_replays(&loop_function, path);
    }
}

#[test]
fn configured_path_limit_is_never_reported_as_complete() {
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
            max_paths: Some(2),
            ..EnumerationOptions::default()
        },
    )
    .unwrap();
    assert_eq!(result.paths.len(), 2);
    assert_eq!(
        result.completeness,
        EnumerationCompleteness::Incomplete(IncompleteReason::PathLimit { limit: 2 })
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
        max_paths: None,
        constraints: vec![InputConstraint::Compare {
            operation: ConstraintComparison::UnsignedLessThan,
            lhs: ConstraintTerm::Input("symex_arg_0".to_owned()),
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
    assert_eq!(result.paths.len(), 2);
    assert_eq!(
        outcomes(&result.paths),
        BTreeSet::from([ChoiceOutcome::Case(0), ChoiceOutcome::Case(1)])
    );
    for path in &result.paths {
        assert!(path.witness.inputs[0].to_u64().unwrap() < 2);
        assert_witness_replays(&function, path);
    }

    let invalid = EnumerationOptions {
        max_paths: None,
        constraints: vec![InputConstraint::Compare {
            operation: ConstraintComparison::Equal,
            lhs: ConstraintTerm::Input("missing".to_owned()),
            rhs: ConstraintTerm::Constant(IrValue::make_ubits(2, 0).unwrap()),
        }],
        ..EnumerationOptions::default()
    };
    let error = enumerate_package_with_options(&package, "select", &invalid).unwrap_err();
    assert!(error.to_string().contains("unknown input leaf \"missing\""));
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
    assert_eq!(result.paths.len(), 4);
    assert!(matches!(result.paths[0].result, SymbolicValue::Tuple(_)));
    for path in &result.paths {
        assert_witness_replays(&function, path);
    }
}
