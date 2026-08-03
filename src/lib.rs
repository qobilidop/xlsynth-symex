// SPDX-License-Identifier: Apache-2.0

//! Symbolic evaluation of pure XLS functions.
//!
//! The evaluator translates supported XLS IR nodes into native symbolic
//! expressions. XLS remains the concrete execution and independent SMT
//! reference boundary used by the validation suite.

mod enumerator;
mod evaluator;
mod expr;
mod solver;

use std::collections::BTreeMap;

use xlsynth::{IrFunction, IrPackage, IrValue, XlsynthError};

/// Specifies which leaves of a function argument are concrete or symbolic.
#[derive(Clone, Debug)]
pub enum EvaluationInput {
    /// Makes every bits leaf in the corresponding type symbolic.
    Symbolic,
    /// Supplies a fully concrete XLS value.
    Concrete(IrValue),
    /// Supplies independently concrete or symbolic tuple elements.
    Tuple(Vec<EvaluationInput>),
    /// Supplies independently concrete or symbolic array elements.
    Array(Vec<EvaluationInput>),
}

/// A symbolic path condition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathCondition(String);

impl PathCondition {
    /// Returns this condition as an SMT-LIB Boolean expression.
    #[must_use]
    pub fn as_smtlib(&self) -> &str {
        &self.0
    }

    fn unconditional() -> Self {
        Self("true".to_owned())
    }

    fn from_smtlib(expression: String) -> Self {
        Self(expression)
    }
}

impl Default for PathCondition {
    fn default() -> Self {
        Self::unconditional()
    }
}

/// The dynamic context that distinguishes repeated evaluations of a choice.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InvocationFrame {
    /// A function invocation at a caller node.
    Invoke {
        /// Calling function name.
        caller: String,
        /// Textual XLS node id of the invocation.
        node_id: usize,
    },
    /// One iteration of a finite `counted_for` at a caller node.
    CountedFor {
        /// Calling function name.
        caller: String,
        /// Textual XLS node id of the loop.
        node_id: usize,
        /// Zero-based dynamic iteration number.
        iteration: usize,
    },
}

/// Stable identity of one choice within a particular evaluation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ChoiceId {
    /// Function containing the choice node.
    pub function: String,
    /// Textual XLS node id.
    pub node_id: usize,
    /// Dynamic invocation and loop context, outermost first.
    pub invocation: Vec<InvocationFrame>,
}

/// Canonical outcome at an active choice node.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChoiceOutcome {
    /// A zero-based selected case.
    Case(usize),
    /// The explicit default arm.
    Default,
    /// The selected low-order case bits of a `one_hot_sel`, LSB first.
    OneHotMask(Vec<bool>),
}

/// Concrete inputs produced by the solver for one feasible path.
#[derive(Clone, Debug)]
pub struct PathWitness {
    /// Complete typed function arguments in signature order.
    pub inputs: Vec<IrValue>,
    /// Model values for symbolic bits leaves, keyed by stable parameter name.
    pub symbolic_leaves: BTreeMap<String, IrValue>,
}

/// One feasible canonical symbolic path.
#[derive(Clone, Debug)]
pub struct PathResult {
    /// Constraint under which this path is active.
    pub condition: PathCondition,
    /// Residual result for this path.
    pub result: SymbolicValue,
    /// Sparse canonical map of active choices to their outcomes.
    pub trace: BTreeMap<ChoiceId, ChoiceOutcome>,
    /// Solver-derived concrete input assignment reaching this path.
    pub witness: PathWitness,
}

/// Why an enumeration returned only a partial result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IncompleteReason {
    /// The caller's configured path limit was reached.
    PathLimit {
        /// Maximum number of feasible paths requested by the caller.
        limit: usize,
    },
    /// An internal safety ceiling prevented impractical materialization.
    ResourceLimit {
        /// Maximum number of syntactic branches considered.
        limit: usize,
        /// Choice that exceeded the ceiling.
        choice: ChoiceId,
    },
    /// A solver invocation failed or returned an indeterminate answer.
    Solver(String),
}

/// Whether all feasible canonical paths were returned.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnumerationCompleteness {
    /// Every feasible path under the v1 policy is present.
    Complete,
    /// Returned paths are useful but do not constitute full coverage.
    Incomplete(IncompleteReason),
}

/// Configuration for canonical path enumeration.
#[derive(Clone, Debug, Default)]
pub struct EnumerationOptions {
    /// Optional maximum number of feasible paths to return.
    ///
    /// `None` performs uncapped exhaustive enumeration. Reaching a configured
    /// limit always reports [`EnumerationCompleteness::Incomplete`].
    pub max_paths: Option<usize>,
}

/// Result of canonical path enumeration.
#[derive(Clone, Debug)]
pub struct EnumerationResult {
    /// Symbolic bits leaves shared by all path expressions.
    pub parameters: Vec<SymbolicParameter>,
    /// Feasible canonical paths in deterministic trace order.
    pub paths: Vec<PathResult>,
    /// Explicit full-coverage status.
    pub completeness: EnumerationCompleteness,
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

/// A structural symbolic XLS value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SymbolicValue {
    /// A bit-vector leaf.
    Bits(SymbolicBits),
    /// A tuple whose elements retain XLS tuple order.
    Tuple(Vec<SymbolicValue>),
    /// A fixed-size array whose elements retain XLS array order.
    Array(Vec<SymbolicValue>),
}

impl SymbolicValue {
    /// Returns this value as bits, or `None` for a structural value.
    #[must_use]
    pub const fn as_bits(&self) -> Option<&SymbolicBits> {
        match self {
            Self::Bits(bits) => Some(bits),
            Self::Tuple(_) | Self::Array(_) => None,
        }
    }

    /// Appends all bits leaves in structural order.
    pub fn flatten_bits<'a>(&'a self, output: &mut Vec<&'a SymbolicBits>) {
        match self {
            Self::Bits(bits) => output.push(bits),
            Self::Tuple(elements) | Self::Array(elements) => {
                for element in elements {
                    element.flatten_bits(output);
                }
            }
        }
    }
}

/// The merged symbolic result of evaluating one XLS function.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymexResult {
    /// Constraint under which the result applies.
    pub path_condition: PathCondition,
    /// Symbolic parameters in function-signature order.
    pub parameters: Vec<SymbolicParameter>,
    /// Native symbolic result expression.
    pub result: SymbolicValue,
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

/// Symbolically evaluates a leaf function with mixed concrete/symbolic inputs.
pub fn evaluate_with_inputs(
    function: &IrFunction,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_function_text_with_inputs(&function.to_ir_string()?, inputs)
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

/// Symbolically evaluates a named function in textual XLS/PIR package IR.
///
/// This boundary also accepts the pinned `xlsynth-pir` extension operations,
/// which are desugared to ordinary XLS value operations before evaluation.
pub fn evaluate_ir_package(
    package_ir: &str,
    function_name: &str,
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text(package_ir, function_name)
}

/// Symbolically evaluates a package function with mixed concrete/symbolic inputs.
pub fn evaluate_package_with_inputs(
    package: &IrPackage,
    function_name: &str,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text_with_inputs(&package.to_string(), function_name, inputs)
}

/// Enumerates every feasible canonical path through a leaf pure XLS function.
pub fn enumerate(function: &IrFunction) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(
        &function.to_ir_string()?,
        None,
        &EnumerationOptions::default(),
    )
}

/// Enumerates canonical paths through a leaf function with mixed inputs.
pub fn enumerate_with_inputs(
    function: &IrFunction,
    inputs: &[EvaluationInput],
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(
        &function.to_ir_string()?,
        Some(inputs),
        &EnumerationOptions::default(),
    )
}

/// Enumerates canonical paths through a leaf function with explicit options.
pub fn enumerate_with_inputs_and_options(
    function: &IrFunction,
    inputs: &[EvaluationInput],
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(&function.to_ir_string()?, Some(inputs), options)
}

/// Enumerates canonical paths through a named function and its owning package.
pub fn enumerate_package(
    package: &IrPackage,
    function_name: &str,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(
        &package.to_string(),
        function_name,
        None,
        &EnumerationOptions::default(),
    )
}

/// Enumerates a named function in textual XLS/PIR package IR.
pub fn enumerate_ir_package(
    package_ir: &str,
    function_name: &str,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(
        package_ir,
        function_name,
        None,
        &EnumerationOptions::default(),
    )
}

/// Enumerates package-function paths with mixed inputs and explicit options.
pub fn enumerate_package_with_inputs_and_options(
    package: &IrPackage,
    function_name: &str,
    inputs: &[EvaluationInput],
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(&package.to_string(), function_name, Some(inputs), options)
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
        let result_bits = result.result.as_bits().unwrap();
        assert_eq!(result_bits.bit_count, 8);
        assert_eq!(result_bits.expression, "(bvadd symex_arg_0 symex_arg_1)");
        assert_ne!(result.result_smtlib, function.to_z3_smtlib().unwrap());
    }

    #[test]
    fn folds_concrete_inputs_and_preserves_symbolic_leaves() {
        let package = IrPackage::parse_ir(ADD_IR, None).unwrap();
        let function = package.get_function("add").unwrap();

        let mixed = evaluate_with_inputs(
            &function,
            &[
                EvaluationInput::Concrete(IrValue::make_ubits(8, 3).unwrap()),
                EvaluationInput::Symbolic,
            ],
        )
        .unwrap();
        assert_eq!(mixed.parameters.len(), 1);
        assert_eq!(mixed.parameters[0].name, "symex_arg_1");
        assert_eq!(
            mixed.result.as_bits().unwrap().expression,
            "(bvadd #b00000011 symex_arg_1)"
        );

        let concrete = evaluate_with_inputs(
            &function,
            &[
                EvaluationInput::Concrete(IrValue::make_ubits(8, 3).unwrap()),
                EvaluationInput::Concrete(IrValue::make_ubits(8, 4).unwrap()),
            ],
        )
        .unwrap();
        assert!(concrete.parameters.is_empty());
        assert_eq!(concrete.result.as_bits().unwrap().expression, "#b00000111");
    }

    #[test]
    fn rejects_mixed_input_shape_mismatches() {
        let package = IrPackage::parse_ir(ADD_IR, None).unwrap();
        let function = package.get_function("add").unwrap();
        let error = evaluate_with_inputs(&function, &[EvaluationInput::Symbolic]).unwrap_err();
        assert!(error.to_string().contains("expects 2 inputs, got 1"));
    }
}
