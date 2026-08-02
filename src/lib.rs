// SPDX-License-Identifier: Apache-2.0

//! Symbolic evaluation of pure XLS functions.
//!
//! The initial implementation is deliberately a minimal vertical slice. It
//! represents every function as one merged SMT result with an unconditional
//! path. Later milestones will replace this adapter with the project's own
//! demand-driven evaluator while preserving the result shape.

use xlsynth::{IrFunction, XlsynthError};

/// A symbolic path condition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathCondition {
    /// The path is unconditional.
    True,
}

impl PathCondition {
    /// Returns this condition as an SMT-LIB Boolean expression.
    #[must_use]
    pub const fn as_smtlib(self) -> &'static str {
        match self {
            Self::True => "true",
        }
    }
}

/// The merged symbolic result of evaluating one XLS function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymexResult {
    /// Constraint under which the result applies.
    pub path_condition: PathCondition,
    /// Complete SMT-LIB encoding of the function result.
    pub result_smtlib: String,
}

/// Symbolically evaluates a pure XLS function as one unconditional result.
///
/// This first implementation intentionally delegates expression construction
/// to XLS's Z3 translator. It establishes the API and differential-testing
/// path before the native symbolic evaluator is introduced.
pub fn evaluate(function: &IrFunction) -> Result<SymexResult, XlsynthError> {
    Ok(SymexResult {
        path_condition: PathCondition::True,
        result_smtlib: function.to_z3_smtlib()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use xlsynth::IrPackage;

    const ADD_IR: &str = r#"package test

top fn add(x: bits[8], y: bits[8]) -> bits[8] {
  ret result: bits[8] = add(x, y)
}
"#;

    #[test]
    fn returns_one_unconditional_upstream_encoding() {
        let package = IrPackage::parse_ir(ADD_IR, None).unwrap();
        let function = package.get_function("add").unwrap();

        let result = evaluate(&function).unwrap();

        assert_eq!(result.path_condition.as_smtlib(), "true");
        assert_eq!(result.result_smtlib, function.to_z3_smtlib().unwrap());
    }
}
