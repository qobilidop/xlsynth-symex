// SPDX-License-Identifier: Apache-2.0

//! Differential semantic microtests for the pinned pure-value operation set.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use xlsynth::{IrBits, IrPackage, IrValue};
use xlsynth_pir::ir_eval::{FnEvalResult, eval_fn};
use xlsynth_pir::ir_parser::Parser;
use xlsynth_symex::{
    EvaluationInput, SymbolicValue, evaluate_ir_package, evaluate_ir_package_with_inputs,
    evaluate_package, evaluate_package_with_inputs,
};

fn bits(width: usize, value: u64) -> IrValue {
    IrValue::make_ubits(width, value).unwrap()
}

fn array(elements: &[IrValue]) -> IrValue {
    IrValue::make_array(elements).unwrap()
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

fn collect_named_ir_bits(value: &IrValue, name: &str, output: &mut BTreeMap<String, IrBits>) {
    if let Ok(elements) = value.get_elements() {
        for (index, element) in elements.iter().enumerate() {
            collect_named_ir_bits(element, &format!("{name}_{index}"), output);
        }
    } else {
        assert!(
            output
                .insert(name.to_owned(), value.to_bits().unwrap())
                .is_none()
        );
    }
}

fn smt_bits(value: &IrBits) -> String {
    let mut result = String::from("#b");
    for index in (0..value.get_bit_count()).rev() {
        result.push(if value.get_bit(index).unwrap() {
            '1'
        } else {
            '0'
        });
    }
    result
}

fn assert_symbolic_matches(ir: &str, function_name: &str, cases: &[Vec<IrValue>]) {
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let function = package.get_function(function_name).unwrap();
    let symbolic = evaluate_package(&package, function_name).unwrap();
    assert!(!cases.is_empty());
    let mut all_symbolic_leaves = Vec::new();
    for arg in &cases[0] {
        flatten_ir_bits(arg, &mut all_symbolic_leaves);
    }
    assert_eq!(
        symbolic.parameters.len(),
        all_symbolic_leaves
            .iter()
            .filter(|bits| bits.get_bit_count() > 0)
            .count()
    );
    for args in cases {
        let expected = function.interpret(args).unwrap();
        assert_symbolic_value(&symbolic.result, &symbolic.parameters, args, &expected);
    }

    let representative = &cases[0];
    let expected = function.interpret(representative).unwrap();
    for concrete_index in 0..representative.len() {
        let inputs = representative
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
        let mixed = evaluate_package_with_inputs(&package, function_name, &inputs).unwrap();
        assert_symbolic_value(&mixed.result, &mixed.parameters, representative, &expected);
    }
}

fn assert_symbolic_value(
    symbolic: &SymbolicValue,
    parameters: &[xlsynth_symex::SymbolicParameter],
    args: &[IrValue],
    expected: &IrValue,
) {
    let mut input_leaves = BTreeMap::new();
    for (index, arg) in args.iter().enumerate() {
        collect_named_ir_bits(arg, &format!("symex_arg_{index}"), &mut input_leaves);
    }
    let bindings = parameters
        .iter()
        .map(|parameter| {
            let value = input_leaves
                .get(&parameter.name)
                .unwrap_or_else(|| panic!("missing concrete value for {}", parameter.name));
            assert_eq!(parameter.bit_count, value.get_bit_count());
            format!("({} {})", parameter.name, smt_bits(value))
        })
        .collect::<Vec<_>>()
        .join(" ");
    let mut symbolic_leaves = Vec::new();
    symbolic.flatten_bits(&mut symbolic_leaves);
    let mut expected_leaves = Vec::new();
    flatten_ir_bits(expected, &mut expected_leaves);
    assert_eq!(symbolic_leaves.len(), expected_leaves.len());
    let equalities = symbolic_leaves
        .iter()
        .zip(&expected_leaves)
        .filter(|(actual, _)| actual.bit_count > 0)
        .map(|(actual, expected)| {
            let expression = if bindings.is_empty() {
                actual.expression.clone()
            } else {
                format!("(let ({bindings}) {})", actual.expression)
            };
            format!("(= {expression} {})", smt_bits(expected))
        })
        .collect::<Vec<_>>();
    let comparison = match equalities.as_slice() {
        [] => "true".to_owned(),
        [only] => only.clone(),
        _ => format!("(and {})", equalities.join(" ")),
    };
    let query = format!("(assert (not {comparison}))\n(check-sat)\n");
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
        "symbolic value differs from XLS\n{query}"
    );
}

#[test]
fn arithmetic_comparison_shift_and_partial_product_operations_match_xls() {
    let binary_cases = vec![
        vec![bits(4, 0), bits(4, 0)],
        vec![bits(4, 1), bits(4, 15)],
        vec![bits(4, 7), bits(4, 3)],
        vec![bits(4, 8), bits(4, 15)],
        vec![bits(4, 15), bits(4, 2)],
    ];
    for (operation, result_type) in [
        ("add", "bits[4]"),
        ("sub", "bits[4]"),
        ("umul", "bits[4]"),
        ("smul", "bits[4]"),
        ("udiv", "bits[4]"),
        ("sdiv", "bits[4]"),
        ("umod", "bits[4]"),
        ("smod", "bits[4]"),
        ("eq", "bits[1]"),
        ("ne", "bits[1]"),
        ("ugt", "bits[1]"),
        ("uge", "bits[1]"),
        ("ult", "bits[1]"),
        ("ule", "bits[1]"),
        ("sgt", "bits[1]"),
        ("sge", "bits[1]"),
        ("slt", "bits[1]"),
        ("sle", "bits[1]"),
    ] {
        let ir = format!(
            "package test\n\ntop fn f(x: bits[4] id=1, y: bits[4] id=2) -> {result_type} {{\n  ret result: {result_type} = {operation}(x, y, id=3)\n}}\n"
        );
        assert_symbolic_matches(&ir, "f", &binary_cases);
    }

    let shift_cases = vec![
        vec![bits(4, 1), bits(8, 0)],
        vec![bits(4, 9), bits(8, 3)],
        vec![bits(4, 9), bits(8, 4)],
        vec![bits(4, 9), bits(8, 16)],
        vec![bits(4, 9), bits(8, 255)],
    ];
    for operation in ["shll", "shrl", "shra"] {
        let ir = format!(
            "package test\n\ntop fn f(x: bits[4] id=1, y: bits[8] id=2) -> bits[4] {{\n  ret result: bits[4] = {operation}(x, y, id=3)\n}}\n"
        );
        assert_symbolic_matches(&ir, "f", &shift_cases);
    }

    for operation in ["umulp", "smulp"] {
        let ir = format!(
            "package test\n\ntop fn f(x: bits[4] id=1, y: bits[4] id=2) -> bits[8] {{\n  parts: (bits[8], bits[8]) = {operation}(x, y, id=3)\n  low: bits[8] = tuple_index(parts, index=0, id=4)\n  high: bits[8] = tuple_index(parts, index=1, id=5)\n  ret result: bits[8] = add(low, high, id=6)\n}}\n"
        );
        assert_symbolic_matches(&ir, "f", &binary_cases);
    }
}

#[test]
fn unary_nary_encoding_and_gate_operations_match_xls() {
    let unary_cases = (0..16)
        .map(|value| vec![bits(4, value)])
        .collect::<Vec<_>>();
    for (operation, result_type) in [
        ("identity", "bits[4]"),
        ("not", "bits[4]"),
        ("neg", "bits[4]"),
        ("reverse", "bits[4]"),
        ("or_reduce", "bits[1]"),
        ("and_reduce", "bits[1]"),
        ("xor_reduce", "bits[1]"),
    ] {
        let ir = format!(
            "package test\n\ntop fn f(x: bits[4] id=1) -> {result_type} {{\n  ret result: {result_type} = {operation}(x, id=2)\n}}\n"
        );
        assert_symbolic_matches(&ir, "f", &unary_cases);
    }
    let nary_cases = vec![
        vec![bits(4, 0), bits(4, 0)],
        vec![bits(4, 3), bits(4, 5)],
        vec![bits(4, 10), bits(4, 12)],
        vec![bits(4, 15), bits(4, 1)],
    ];
    for operation in ["and", "nand", "nor", "or", "xor"] {
        let ir = format!(
            "package test\n\ntop fn f(x: bits[4] id=1, y: bits[4] id=2) -> bits[4] {{\n  ret result: bits[4] = {operation}(x, y, x, id=3)\n}}\n"
        );
        assert_symbolic_matches(&ir, "f", &nary_cases);
    }

    let concat_ir = r#"package test

top fn f(x: bits[4] id=1, y: bits[4] id=2) -> bits[8] {
  ret result: bits[8] = concat(x, y, id=3)
}
"#;
    assert_symbolic_matches(concat_ir, "f", &nary_cases);

    let one_hot_ir = r#"package test

top fn f(x: bits[4] id=1) -> (bits[5], bits[5], bits[2], bits[4]) {
  lsb: bits[5] = one_hot(x, lsb_prio=true, id=2)
  msb: bits[5] = one_hot(x, lsb_prio=false, id=3)
  encoded: bits[2] = encode(x, id=4)
  decoded: bits[4] = decode(encoded, width=4, id=5)
  ret result: (bits[5], bits[5], bits[2], bits[4]) = tuple(lsb, msb, encoded, decoded, id=6)
}
"#;
    assert_symbolic_matches(one_hot_ir, "f", &unary_cases);

    let gate_ir = r#"package test

top fn f(predicate: bits[1] id=1, x: (bits[4], bits[4]) id=2) -> (bits[4], bits[4]) {
  ret result: (bits[4], bits[4]) = gate(predicate, x, id=3)
}
"#;
    let tuples = [
        IrValue::make_tuple(&[bits(4, 3), bits(4, 12)]),
        IrValue::make_tuple(&[bits(4, 15), bits(4, 1)]),
    ];
    let gate_cases = vec![
        vec![bits(1, 0), tuples[0].clone()],
        vec![bits(1, 1), tuples[0].clone()],
        vec![bits(1, 0), tuples[1].clone()],
        vec![bits(1, 1), tuples[1].clone()],
    ];
    assert_symbolic_matches(gate_ir, "f", &gate_cases);
}

#[test]
fn slicing_update_and_extension_operations_match_xls() {
    let slice_ir = r#"package test

top fn f(x: bits[16] id=1, start: bits[16] id=2, update: bits[5] id=3) -> (bits[3], bits[12], bits[16], bits[20], bits[4]) {
  fixed: bits[3] = bit_slice(x, start=2, width=3, id=4)
  dynamic: bits[12] = dynamic_bit_slice(x, start, width=12, id=5)
  updated: bits[16] = bit_slice_update(x, start, update, id=6)
  zeroed: bits[20] = zero_ext(x, new_bit_count=20, id=7)
  signed: bits[4] = sign_ext(fixed, new_bit_count=4, id=8)
  ret result: (bits[3], bits[12], bits[16], bits[20], bits[4]) = tuple(fixed, dynamic, updated, zeroed, signed, id=9)
}
"#;
    let cases = vec![
        vec![bits(16, 0), bits(16, 0), bits(5, 31)],
        vec![bits(16, 0xa5a5), bits(16, 2), bits(5, 0x12)],
        vec![bits(16, 0xffff), bits(16, 7), bits(5, 0)],
        vec![bits(16, 0x8101), bits(16, 16), bits(5, 31)],
        vec![bits(16, 0x8101), bits(16, 256), bits(5, 31)],
        vec![bits(16, 0x8101), bits(16, 65_535), bits(5, 31)],
    ];
    assert_symbolic_matches(slice_ir, "f", &cases);
}

#[test]
fn nested_array_tuple_index_concat_slice_and_update_operations_match_xls() {
    let ir = r#"package test

top fn f(a: bits[4][2][2] id=1, b: bits[4][2] id=2, outer: bits[3] id=3, inner: bits[3] id=4, value: bits[4] id=5, start: bits[8] id=6) -> (bits[4], bits[4][2][2], bits[4][4], bits[4][3], bits[4]) {
  indexed: bits[4] = array_index(a, indices=[outer, inner], assumed_in_bounds=false, id=7)
  updated: bits[4][2][2] = array_update(a, value, indices=[outer, inner], assumed_in_bounds=false, id=8)
  first: bits[4][2] = array_index(a, indices=[outer], assumed_in_bounds=false, id=9)
  joined: bits[4][4] = array_concat(first, b, id=10)
  sliced: bits[4][3] = array_slice(b, start, width=3, id=11)
  pair: (bits[4], bits[4]) = tuple(indexed, value, id=12)
  projected: bits[4] = tuple_index(pair, index=1, id=13)
  ret result: (bits[4], bits[4][2][2], bits[4][4], bits[4][3], bits[4]) = tuple(indexed, updated, joined, sliced, projected, id=14)
}
"#;
    let nested = array(&[
        array(&[bits(4, 1), bits(4, 2)]),
        array(&[bits(4, 3), bits(4, 4)]),
    ]);
    let second = array(&[bits(4, 10), bits(4, 11)]);
    let cases = vec![
        vec![
            nested.clone(),
            second.clone(),
            bits(3, 0),
            bits(3, 1),
            bits(4, 9),
            bits(8, 0),
        ],
        vec![
            nested.clone(),
            second.clone(),
            bits(3, 1),
            bits(3, 0),
            bits(4, 9),
            bits(8, 1),
        ],
        vec![
            nested.clone(),
            second.clone(),
            bits(3, 7),
            bits(3, 7),
            bits(4, 9),
            bits(8, 2),
        ],
        vec![
            nested,
            second,
            bits(3, 7),
            bits(3, 7),
            bits(4, 9),
            bits(8, 255),
        ],
    ];
    assert_symbolic_matches(ir, "f", &cases);
}

#[test]
fn pir_extension_operations_desugar_before_symbolic_evaluation() {
    let ir = r#"package test

top fn f(x: bits[8] id=1, y: bits[8] id=2, carry: bits[1] id=3, count: bits[4] id=4) -> (bits[1], bits[4], bits[4], (bits[12], bits[4]), bits[8], bits[10]) {
  carry_out: bits[1] = ext_carry_out(x, y, carry, id=5)
  encoded: bits[4] = ext_prio_encode(x, lsb_prio=true, id=6)
  leading: bits[4] = ext_clz(x, offset=1, new_bit_count=4, id=7)
  normalized: (bits[12], bits[4]) = ext_normalize_left(x, shift_offset=1, normalized_bit_count=12, clz_bit_count=4, id=8)
  mask: bits[8] = ext_mask_low(count, id=9)
  sum: bits[10] = ext_nary_add(x, y, signed=[false, true], negated=[false, true], id=10)
  ret result: (bits[1], bits[4], bits[4], (bits[12], bits[4]), bits[8], bits[10]) = tuple(carry_out, encoded, leading, normalized, mask, sum, id=11)
}
"#;
    let symbolic = evaluate_ir_package(ir, "f").unwrap();
    assert_eq!(symbolic.parameters.len(), 4);
    assert_eq!(
        symbolic.result.as_bits(),
        None,
        "extension bundle returns a tuple"
    );
    let package = Parser::new(ir).parse_and_validate_package().unwrap();
    let function = package.get_fn("f").unwrap();
    for args in [
        vec![bits(8, 0), bits(8, 0), bits(1, 0), bits(4, 0)],
        vec![bits(8, 0xff), bits(8, 1), bits(1, 0), bits(4, 8)],
        vec![bits(8, 0x81), bits(8, 0x7f), bits(1, 1), bits(4, 15)],
        vec![bits(8, 0x24), bits(8, 0xe1), bits(1, 1), bits(4, 3)],
    ] {
        let expected = match eval_fn(function, &args) {
            FnEvalResult::Success(success) => success.value,
            FnEvalResult::Failure(failure) => panic!("PIR evaluation failed: {failure:?}"),
        };
        assert_symbolic_value(&symbolic.result, &symbolic.parameters, &args, &expected);
    }

    let representative = vec![bits(8, 0x24), bits(8, 0xe1), bits(1, 1), bits(4, 3)];
    let expected = match eval_fn(function, &representative) {
        FnEvalResult::Success(success) => success.value,
        FnEvalResult::Failure(failure) => panic!("PIR evaluation failed: {failure:?}"),
    };
    for concrete_index in 0..representative.len() {
        let inputs = representative
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
        let mixed = evaluate_ir_package_with_inputs(ir, "f", &inputs).unwrap();
        assert_symbolic_value(&mixed.result, &mixed.parameters, &representative, &expected);
    }
}

#[test]
fn zero_width_value_operations_preserve_xls_semantics() {
    let ir = r#"package test

top fn f(x: bits[0] id=1, y: bits[0] id=2) -> (bits[0], bits[1], bits[1], bits[0]) {
  sum: bits[0] = add(x, y, id=3)
  equal: bits[1] = eq(x, y, id=4)
  less: bits[1] = ult(x, y, id=5)
  parts: (bits[0], bits[0]) = umulp(x, y, id=6)
  part0: bits[0] = tuple_index(parts, index=0, id=7)
  part1: bits[0] = tuple_index(parts, index=1, id=8)
  product: bits[0] = add(part0, part1, id=9)
  ret result: (bits[0], bits[1], bits[1], bits[0]) = tuple(sum, equal, less, product, id=10)
}
"#;
    assert_symbolic_matches(ir, "f", &[vec![bits(0, 0), bits(0, 0)]]);
}

#[test]
fn arbitrary_width_parameters_and_literals_are_native_values() {
    let ir = r#"package test

top fn f(x: bits[130] id=1) -> bits[130] {
  one: bits[130] = literal(value=1, id=2)
  ret result: bits[130] = add(x, one, id=3)
}
"#;
    let patterned = IrBits::from_lsb_is_0(
        &(0..130)
            .map(|index| index % 3 == 0 || index == 129)
            .collect::<Vec<_>>(),
    );
    assert_symbolic_matches(
        ir,
        "f",
        &[
            vec![IrValue::from_bits(&IrBits::zero(130))],
            vec![IrValue::from_bits(&IrBits::all_ones(130))],
            vec![IrValue::from_bits(&patterned)],
        ],
    );
}
