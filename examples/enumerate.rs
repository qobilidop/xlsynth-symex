// SPDX-License-Identifier: Apache-2.0

//! Minimal complete-path enumeration example.

use xlsynth::IrPackage;
use xlsynth_symex::{EnumerationCompleteness, enumerate_package};

fn main() {
    let ir = r#"package example

top fn choose(selector: bits[2] id=1, a: bits[8] id=2, b: bits[8] id=3, fallback: bits[8] id=4) -> bits[8] {
  ret result: bits[8] = sel(selector, cases=[a, b], default=fallback, id=5)
}
"#;
    let package = IrPackage::parse_ir(ir, None).expect("example IR must parse");
    let result = enumerate_package(&package, "choose").expect("enumeration must run");

    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    println!("{} feasible paths", result.paths.len());
    for path in result.paths {
        println!("{:?}: {}", path.trace, path.condition.as_smtlib());
    }
}
