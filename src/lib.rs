// SPDX-License-Identifier: Apache-2.0

//! Symbolic evaluation of pure XLS functions.
//!
//! The evaluator translates supported XLS IR nodes into native symbolic
//! expressions. XLS remains the concrete execution and independent SMT
//! reference boundary used by the validation suite.

mod evaluator;

use xlsynth::{IrFunction, IrPackage, XlsynthError};

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

/// One symbolic bits-typed function parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolicParameter {
    /// Stable SMT-LIB identifier assigned by parameter position.
    pub name: String,
    /// Parameter width in bits.
    pub bit_count: usize,
}

/// A bits-typed symbolic value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolicBits {
    /// Width of the expression in bits.
    pub bit_count: usize,
    /// SMT-LIB bit-vector expression.
    pub expression: String,
}

/// The merged symbolic result of evaluating one XLS function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymexResult {
    /// Constraint under which the result applies.
    pub path_condition: PathCondition,
    /// Symbolic parameters in function-signature order.
    pub parameters: Vec<SymbolicParameter>,
    /// Native symbolic result expression.
    pub result: SymbolicBits,
    /// Self-contained SMT-LIB declarations for the parameters and result.
    pub result_smtlib: String,
}

/// Symbolically evaluates a leaf pure XLS function.
///
/// This entry point parses the function's standalone IR text. Functions that
/// invoke other functions require [`evaluate_package`] so callees are present.
pub fn evaluate(function: &IrFunction) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_function_text(&function.to_ir_string()?)
}

/// Symbolically evaluates a named pure XLS function with its owning package.
///
/// Package evaluation supports calls to other functions in the same package.
pub fn evaluate_package(
    package: &IrPackage,
    function_name: &str,
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text(&package.to_string(), function_name)
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
    fn builds_native_add_expression() {
        let package = IrPackage::parse_ir(ADD_IR, None).unwrap();
        let function = package.get_function("add").unwrap();

        let result = evaluate(&function).unwrap();

        assert_eq!(result.path_condition.as_smtlib(), "true");
        assert_eq!(
            result.parameters,
            vec![
                SymbolicParameter {
                    name: "symex_arg_0".to_owned(),
                    bit_count: 8,
                },
                SymbolicParameter {
                    name: "symex_arg_1".to_owned(),
                    bit_count: 8,
                },
            ]
        );
        assert_eq!(result.result.bit_count, 8);
        assert_eq!(result.result.expression, "(bvadd symex_arg_0 symex_arg_1)");
        assert_ne!(result.result_smtlib, function.to_z3_smtlib().unwrap());
    }
}
