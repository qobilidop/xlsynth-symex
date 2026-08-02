// SPDX-License-Identifier: Apache-2.0

//! Semantic differential tests for the upstream-backed vertical slice.

use std::io::Write;
use std::process::{Command, Stdio};

use xlsynth::{IrPackage, IrValue};
use xlsynth_symex::evaluate;

fn assert_smt_result(function_name: &str, smtlib: &str, width: usize, args: &[u64], expected: u64) {
    let mut application = format!("(select {function_name}");
    for arg in args {
        application.push_str(&format!(" (_ bv{arg} {width})"));
    }
    application.push(')');

    let query = format!(
        "{smtlib}\n(assert (not (= {application} (_ bv{expected} {width}))))\n(check-sat)\n"
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
        .unwrap()
        .write_all(query.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "z3 failed\nstdout: {}\nstderr: {}\nquery:\n{query}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "unsat",
        "SMT result differs from interpreter\nquery:\n{query}"
    );
}

#[test]
fn smt_result_matches_interpreter_for_add() {
    let ir = r#"package test

top fn add(x: bits[8], y: bits[8]) -> bits[8] {
  ret result: bits[8] = add(x, y)
}
"#;
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let function = package.get_function("add").unwrap();
    let result = evaluate(&function).unwrap();

    for (x, y) in [(0, 0), (1, 2), (255, 1), (128, 255)] {
        let args = [
            IrValue::make_ubits(8, x).unwrap(),
            IrValue::make_ubits(8, y).unwrap(),
        ];
        let expected = function.interpret(&args).unwrap().to_u64().unwrap();
        assert_smt_result("add", &result.result_smtlib, 8, &[x, y], expected);
    }
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    And,
    Or,
    Xor,
    Not,
    Neg,
}

impl Op {
    const ALL: [Self; 7] = [
        Self::Add,
        Self::Sub,
        Self::And,
        Self::Or,
        Self::Xor,
        Self::Not,
        Self::Neg,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Sub => "sub",
            Self::And => "and",
            Self::Or => "or",
            Self::Xor => "xor",
            Self::Not => "not",
            Self::Neg => "neg",
        }
    }

    const fn is_unary(self) -> bool {
        matches!(self, Self::Not | Self::Neg)
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

fn generate_function(rng: &mut DeterministicRng, width: usize, node_count: usize) -> String {
    let mut operands = vec!["x".to_owned(), "y".to_owned()];
    let mut body = String::new();
    for index in 0..node_count {
        let op = Op::ALL[rng.next() as usize % Op::ALL.len()];
        let lhs = &operands[rng.next() as usize % operands.len()];
        let expression = if op.is_unary() {
            format!("{}({lhs})", op.name())
        } else {
            let rhs = &operands[rng.next() as usize % operands.len()];
            format!("{}({lhs}, {rhs})", op.name())
        };
        let name = format!("n{index}");
        body.push_str(&format!("  {name}: bits[{width}] = {expression}\n"));
        operands.push(name);
    }
    let result = operands.last().unwrap();
    format!(
        "package fuzz\n\ntop fn fuzz(x: bits[{width}], y: bits[{width}]) -> bits[{width}] {{\n{body}  ret result: bits[{width}] = identity({result})\n}}\n"
    )
}

#[test]
fn deterministic_ir_fuzz_matches_interpreter() {
    const CASES: usize = 256;
    let mut rng = DeterministicRng::new(0x5eed_5eed_cafe_f00d);

    for case in 0..CASES {
        let width = 1 + rng.next() as usize % 16;
        let node_count = 1 + rng.next() as usize % 20;
        let ir = generate_function(&mut rng, width, node_count);
        let mask = (1_u64 << width) - 1;
        let x = rng.next() & mask;
        let y = rng.next() & mask;

        let package = IrPackage::parse_ir(&ir, None).unwrap_or_else(|error| {
            panic!("case {case}: failed to parse generated IR: {error}\n{ir}")
        });
        let function = package.get_function("fuzz").unwrap();
        let args = [
            IrValue::make_ubits(width, x).unwrap(),
            IrValue::make_ubits(width, y).unwrap(),
        ];
        let expected = function.interpret(&args).unwrap().to_u64().unwrap();
        let result = evaluate(&function).unwrap();

        assert_smt_result("fuzz", &result.result_smtlib, width, &[x, y], expected);
    }
}
