// SPDX-License-Identifier: Apache-2.0

//! Symbolic evaluation and exhaustive path generation for pure XLS functions.
//!
//! Canonical enumeration returns every feasible selection trace, its condition
//! and residual value, and a concrete witness, with explicit complete or
//! incomplete status. XLS remains the concrete execution and independent SMT
//! reference boundary used by the validation suite.

mod enumerator;
mod evaluator;
mod expr;
mod solver;

use std::collections::BTreeMap;
use std::time::Duration;

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

/// Structural identity of one bits leaf in the function arguments.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InputLeaf {
    argument_index: usize,
    element_path: Vec<usize>,
}

impl InputLeaf {
    /// Identifies a bits-typed argument with no structural descent.
    #[must_use]
    pub const fn argument(argument_index: usize) -> Self {
        Self {
            argument_index,
            element_path: Vec::new(),
        }
    }

    /// Identifies a leaf by argument index and tuple/array element indices.
    #[must_use]
    pub const fn new(argument_index: usize, element_path: Vec<usize>) -> Self {
        Self {
            argument_index,
            element_path,
        }
    }

    /// Descends into one tuple or array element.
    #[must_use]
    pub fn element(mut self, index: usize) -> Self {
        self.element_path.push(index);
        self
    }

    /// Returns the zero-based function argument index.
    #[must_use]
    pub const fn argument_index(&self) -> usize {
        self.argument_index
    }

    /// Returns tuple/array element indices from the argument root to the leaf.
    #[must_use]
    pub fn element_path(&self) -> &[usize] {
        &self.element_path
    }
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
    /// Model values keyed by complete symbolic-parameter metadata.
    pub symbolic_leaves: BTreeMap<SymbolicParameter, IrValue>,
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

/// Backend-neutral bit-vector term used in caller assumptions.
#[derive(Clone, Debug)]
pub enum ConstraintTerm {
    /// A symbolic bits leaf identified independently of solver rendering.
    Input(InputLeaf),
    /// A bits-typed XLS constant.
    Constant(IrValue),
    /// Bitwise complement.
    Not(Box<ConstraintTerm>),
    /// Modular addition of equal-width terms.
    Add(Box<ConstraintTerm>, Box<ConstraintTerm>),
    /// Modular subtraction of equal-width terms.
    Sub(Box<ConstraintTerm>, Box<ConstraintTerm>),
    /// Bitwise conjunction of equal-width terms.
    And(Box<ConstraintTerm>, Box<ConstraintTerm>),
    /// Bitwise disjunction of equal-width terms.
    Or(Box<ConstraintTerm>, Box<ConstraintTerm>),
    /// Bitwise exclusive-or of equal-width terms.
    Xor(Box<ConstraintTerm>, Box<ConstraintTerm>),
}

/// Comparison used by a caller-supplied input constraint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstraintComparison {
    /// Equal bit patterns.
    Equal,
    /// Different bit patterns.
    NotEqual,
    /// Unsigned less than.
    UnsignedLessThan,
    /// Unsigned less than or equal.
    UnsignedLessOrEqual,
    /// Signed less than.
    SignedLessThan,
    /// Signed less than or equal.
    SignedLessOrEqual,
}

/// Backend-neutral Boolean constraint over symbolic input leaves.
#[derive(Clone, Debug)]
pub enum InputConstraint {
    /// Constant Boolean value.
    Bool(bool),
    /// Logical negation.
    Not(Box<InputConstraint>),
    /// Logical conjunction.
    And(Vec<InputConstraint>),
    /// Logical disjunction.
    Or(Vec<InputConstraint>),
    /// Equal-width bit-vector comparison.
    Compare {
        /// Comparison operation.
        operation: ConstraintComparison,
        /// Left-hand bit-vector term.
        lhs: ConstraintTerm,
        /// Right-hand bit-vector term.
        rhs: ConstraintTerm,
    },
}

/// Configuration for canonical path enumeration.
#[derive(Clone, Debug)]
pub struct EnumerationOptions {
    /// Optional maximum number of feasible paths to return.
    ///
    /// `None` performs uncapped exhaustive enumeration. Reaching a configured
    /// limit always reports [`EnumerationCompleteness::Incomplete`].
    pub max_paths: Option<usize>,
    /// Caller assumptions conjoined with every path condition.
    ///
    /// Constraints use structural [`InputLeaf`] identifiers. A completed result
    /// covers exactly the domain satisfying these assumptions.
    pub constraints: Vec<InputConstraint>,
    /// Per-query solver timeout.
    ///
    /// A timeout produces an explicitly incomplete result. The default is ten
    /// seconds per feasibility/model query.
    pub solver_timeout: Duration,
}

impl Default for EnumerationOptions {
    fn default() -> Self {
        Self {
            max_paths: None,
            constraints: Vec::new(),
            solver_timeout: Duration::from_secs(10),
        }
    }
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
    /// Measurements from constructing and solving this enumeration.
    pub statistics: EnumerationStatistics,
}

/// Measurements for one canonical path-enumeration request.
#[derive(Clone, Debug, Default)]
pub struct EnumerationStatistics {
    /// Unique nodes in the interned symbolic-expression DAG.
    pub expression_nodes: usize,
    /// Function nodes evaluated on cache misses across all path states.
    pub evaluated_nodes: usize,
    /// Function-node values reused from a path-local memoization cache.
    pub cache_hits: usize,
    /// Demanded choice sites resolved without symbolic forking.
    pub concrete_choices: usize,
    /// Symbolic choice outcomes constructed before feasibility pruning.
    pub symbolic_outcomes: usize,
    /// Feasibility and model queries issued to the solver.
    pub solver_queries: usize,
    /// Syntactic candidates rejected as infeasible by the solver.
    pub infeasible_candidates: usize,
    /// Time spent constructing symbolic candidates, excluding solver queries.
    pub construction_time: Duration,
    /// Aggregate wall time spent in solver queries.
    pub solver_time: Duration,
}

/// One symbolic bits-typed function parameter.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SymbolicParameter {
    /// Structural input leaf represented by this parameter.
    pub input: InputLeaf,
    /// Stable SMT-LIB identifier derived from argument and element positions.
    pub name: String,
    /// Parameter width in bits.
    pub bit_count: usize,
}

/// A bits-typed symbolic value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolicBits {
    /// Width of the expression in bits.
    pub bit_count: usize,
    /// SMT-LIB bit-vector expression, empty only for `bits[0]`.
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
    /// Self-contained SMT-LIB declarations for parameters and nonzero result leaves.
    ///
    /// A bits-typed result is named `xlsynth_symex_result`. Structural result
    /// leaves are named `xlsynth_symex_result_leaf_N` in flattening order.
    pub result_smtlib: String,
}

/// Symbolically evaluates a leaf pure XLS function.
///
/// This entry point parses the function's standalone IR text. Functions that
/// invoke other functions require [`evaluate_package`] so callees are present.
///
/// # Errors
///
/// Returns an error if IR conversion or validation fails, or if the function
/// contains a value shape or operation outside the supported pure-function scope.
pub fn evaluate(function: &IrFunction) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_function_text(&function.to_ir_string()?)
}

/// Symbolically evaluates a leaf function with mixed concrete/symbolic inputs.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`evaluate`], or when
/// the supplied inputs do not match the function signature.
pub fn evaluate_with_inputs(
    function: &IrFunction,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_function_text_with_inputs(&function.to_ir_string()?, inputs)
}

/// Symbolically evaluates a named pure XLS function with its owning package.
///
/// Package evaluation supports calls to other functions in the same package.
///
/// # Errors
///
/// Returns an error if the function is absent, package validation fails, or
/// evaluation reaches an unsupported value shape or operation.
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
///
/// # Errors
///
/// Returns an error if parsing, validation, extension desugaring, function
/// lookup, or symbolic evaluation fails.
pub fn evaluate_ir_package(
    package_ir: &str,
    function_name: &str,
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text(package_ir, function_name)
}

/// Symbolically evaluates textual XLS/PIR package IR with mixed inputs.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`evaluate_ir_package`],
/// or when the supplied inputs do not match the function signature.
pub fn evaluate_ir_package_with_inputs(
    package_ir: &str,
    function_name: &str,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text_with_inputs(package_ir, function_name, inputs)
}

/// Symbolically evaluates a package function with mixed concrete/symbolic inputs.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`evaluate_package`], or
/// when the supplied inputs do not match the function signature.
pub fn evaluate_package_with_inputs(
    package: &IrPackage,
    function_name: &str,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluator::evaluate_package_text_with_inputs(&package.to_string(), function_name, inputs)
}

/// Enumerates every feasible canonical path through a leaf pure XLS function.
///
/// # Errors
///
/// Returns an error if IR conversion, path construction, or witness
/// reconstruction fails. Solver indeterminacy is reported as incompleteness.
pub fn enumerate(function: &IrFunction) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(
        &function.to_ir_string()?,
        None,
        &EnumerationOptions::default(),
    )
}

/// Enumerates canonical paths through an all-symbolic leaf function with options.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate`], or when
/// a caller constraint is ill-typed or references a non-symbolic leaf.
pub fn enumerate_with_options(
    function: &IrFunction,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(&function.to_ir_string()?, None, options)
}

/// Enumerates canonical paths through a leaf function with mixed inputs.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate`], or when
/// the supplied inputs do not match the function signature.
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
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate_with_inputs`]
/// or [`enumerate_with_options`].
pub fn enumerate_with_inputs_and_options(
    function: &IrFunction,
    inputs: &[EvaluationInput],
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_function_text(&function.to_ir_string()?, Some(inputs), options)
}

/// Enumerates canonical paths through a named function and its owning package.
///
/// # Errors
///
/// Returns an error if the function is absent, package conversion fails, or
/// path construction or witness reconstruction fails.
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

/// Enumerates an all-symbolic package function with explicit options.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate_package`],
/// or when a caller constraint is invalid.
pub fn enumerate_package_with_options(
    package: &IrPackage,
    function_name: &str,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(&package.to_string(), function_name, None, options)
}

/// Enumerates a named function in textual XLS/PIR package IR.
///
/// # Errors
///
/// Returns an error if parsing, validation, extension desugaring, function
/// lookup, path construction, or witness reconstruction fails.
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

/// Enumerates all-symbolic paths in textual XLS/PIR package IR with options.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate_ir_package`],
/// or when a caller constraint is invalid.
pub fn enumerate_ir_package_with_options(
    package_ir: &str,
    function_name: &str,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(package_ir, function_name, None, options)
}

/// Enumerates package-function paths with mixed inputs and default options.
///
/// # Errors
///
/// Returns an error under the conditions documented by [`enumerate_package`],
/// or when the supplied inputs do not match the function signature.
pub fn enumerate_package_with_inputs(
    package: &IrPackage,
    function_name: &str,
    inputs: &[EvaluationInput],
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(
        &package.to_string(),
        function_name,
        Some(inputs),
        &EnumerationOptions::default(),
    )
}

/// Enumerates package-function paths with mixed inputs and explicit options.
///
/// # Errors
///
/// Returns an error under the conditions documented by
/// [`enumerate_package_with_inputs`] or [`enumerate_package_with_options`].
pub fn enumerate_package_with_inputs_and_options(
    package: &IrPackage,
    function_name: &str,
    inputs: &[EvaluationInput],
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(&package.to_string(), function_name, Some(inputs), options)
}

/// Enumerates textual package-function paths with mixed inputs and options.
///
/// # Errors
///
/// Returns an error under the conditions documented by
/// [`enumerate_ir_package_with_options`], or when the supplied inputs do not
/// match the function signature.
pub fn enumerate_ir_package_with_inputs_and_options(
    package_ir: &str,
    function_name: &str,
    inputs: &[EvaluationInput],
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    enumerator::enumerate_package_text(package_ir, function_name, Some(inputs), options)
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
                    input: InputLeaf::argument(0),
                    name: "symex_arg_0".to_owned(),
                    bit_count: 8,
                },
                SymbolicParameter {
                    input: InputLeaf::argument(1),
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
        assert_eq!(mixed.parameters[0].input, InputLeaf::argument(1));
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
    fn emits_named_definitions_for_structural_result_leaves() {
        let ir = r"package test

top fn pair(x: bits[8]) -> (bits[8], bits[8]) {
  complement: bits[8] = not(x)
  ret result: (bits[8], bits[8]) = tuple(x, complement)
}
";
        let package = IrPackage::parse_ir(ir, None).unwrap();
        let function = package.get_function("pair").unwrap();

        let result = evaluate(&function).unwrap();

        assert_eq!(
            result.result_smtlib,
            "(declare-const symex_arg_0 (_ BitVec 8))\n\
             (define-fun xlsynth_symex_result_leaf_0 () (_ BitVec 8) symex_arg_0)\n\
             (define-fun xlsynth_symex_result_leaf_1 () (_ BitVec 8) (bvnot symex_arg_0))\n"
        );
    }

    #[test]
    fn rejects_mixed_input_shape_mismatches() {
        let package = IrPackage::parse_ir(ADD_IR, None).unwrap();
        let function = package.get_function("add").unwrap();
        let error = evaluate_with_inputs(&function, &[EvaluationInput::Symbolic]).unwrap_err();
        assert!(error.to_string().contains("expects 2 inputs, got 1"));
    }
}
