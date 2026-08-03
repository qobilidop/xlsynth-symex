// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, Instant};

use xlsynth::{IrBits, IrValue, XlsynthError};
use xlsynth_pir::ir::{Fn, NodePayload, NodeRef, Package, Type};
use xlsynth_pir::ir_parser::Parser;
use xlsynth_pir::ir_utils::operands;

use crate::evaluator::{
    BitsValue, Value, apply_pure_node, bits, bits_expr, deep_or, evaluation_input,
    materialize_value, selector_equals, symbolic_input, symex_error, zero_value,
};
use crate::expr::{BitBinaryOp, BitUnaryOp, CompareOp, ExprArena, ExprId, Sort};
use crate::solver::{self, Satisfiability};
use crate::{
    ChoiceId, ChoiceOutcome, ConstraintComparison, ConstraintTerm, EnumerationCompleteness,
    EnumerationOptions, EnumerationResult, EnumerationStatistics, EvaluationInput,
    IncompleteReason, InputConstraint, InvocationFrame, PathCondition, PathResult, PathWitness,
    SymbolicParameter,
};

const SYNTACTIC_PATH_SAFETY_LIMIT: usize = 1_000_000;

pub(crate) fn enumerate_function_text(
    function_text: &str,
    inputs: Option<&[EvaluationInput]>,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    let package_text = format!("package standalone\n\n{function_text}");
    let mut package = parse_package(&package_text)?;
    let function_name = package
        .get_top_fn()
        .ok_or_else(|| symex_error("standalone IR has no function"))?
        .name
        .clone();
    enumerate_parsed(&mut package, &function_name, inputs, options)
}

pub(crate) fn enumerate_package_text(
    package_text: &str,
    function_name: &str,
    inputs: Option<&[EvaluationInput]>,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    let mut package = parse_package(package_text)?;
    enumerate_parsed(&mut package, function_name, inputs, options)
}

fn parse_package(text: &str) -> Result<Package, XlsynthError> {
    let mut package = Parser::new(text)
        .parse_and_validate_package()
        .map_err(|error| symex_error(format!("failed to parse XLS IR package: {error}")))?;
    xlsynth_pir::desugar_extensions::desugar_extensions_in_package(&mut package)
        .map_err(|error| symex_error(format!("failed to desugar extension operations: {error}")))?;
    Ok(package)
}

fn enumerate_parsed(
    package: &mut Package,
    function_name: &str,
    inputs: Option<&[EvaluationInput]>,
    options: &EnumerationOptions,
) -> Result<EnumerationResult, XlsynthError> {
    let construction_started = Instant::now();
    let function = package
        .get_fn(function_name)
        .ok_or_else(|| symex_error(format!("function {function_name:?} is absent from package")))?;
    if let Some(inputs) = inputs
        && inputs.len() != function.params.len()
    {
        return Err(symex_error(format!(
            "function {function_name:?} expects {} inputs, got {}",
            function.params.len(),
            inputs.len()
        )));
    }

    let mut arena = ExprArena::default();
    let mut parameters = Vec::new();
    let arguments = function
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let name = format!("symex_arg_{index}");
            match inputs.map(|inputs| &inputs[index]) {
                Some(input) => {
                    evaluation_input(&mut arena, &parameter.ty, input, &name, &mut parameters)
                }
                None => symbolic_input(&mut arena, &parameter.ty, &name, &mut parameters),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;
    let assumptions = options
        .constraints
        .iter()
        .map(|constraint| lower_constraint(&mut arena, &parameters, constraint))
        .collect::<Result<Vec<_>, _>>()?;
    let initial_condition = arena.bool_and(assumptions);
    let frame = Frame {
        function,
        arguments,
        invocation: Vec::new(),
    };
    let initial = State {
        condition: initial_condition,
        trace: BTreeMap::new(),
        cache: HashMap::new(),
    };
    let mut evaluator = PathEvaluator {
        package,
        arena: &mut arena,
        syntactic_paths: 1,
        incomplete: None,
        evaluated_nodes: 0,
        cache_hits: 0,
        concrete_choices: 0,
        symbolic_outcomes: 0,
    };
    let ret = function
        .ret_node_ref
        .ok_or_else(|| symex_error(format!("function {} has no return node", function.name)))?;
    let candidates = evaluator.eval_node(&frame, ret, initial)?;
    let construction_time = construction_started.elapsed();
    let evaluated_nodes = evaluator.evaluated_nodes;
    let cache_hits = evaluator.cache_hits;
    let concrete_choices = evaluator.concrete_choices;
    let symbolic_outcomes = evaluator.symbolic_outcomes;
    let incomplete = evaluator.incomplete.take();
    drop(evaluator);
    let mut completeness = incomplete
        .map(EnumerationCompleteness::Incomplete)
        .unwrap_or(EnumerationCompleteness::Complete);

    let mut paths = Vec::new();
    let mut seen_traces = HashSet::new();
    let mut solver_queries = 0;
    let mut infeasible_candidates = 0;
    let mut solver_time = Duration::ZERO;
    for candidate in candidates {
        let condition = arena.to_smtlib(candidate.state.condition);
        solver_queries += 1;
        let solver_started = Instant::now();
        let solved = solver::solve(&parameters, &condition, options.solver_timeout);
        solver_time += solver_started.elapsed();
        let model = match solved {
            Ok(Satisfiability::Sat(model)) => model,
            Ok(Satisfiability::Unsat) => {
                infeasible_candidates += 1;
                continue;
            }
            Ok(Satisfiability::Indeterminate(reason)) => {
                completeness =
                    EnumerationCompleteness::Incomplete(IncompleteReason::Solver(reason));
                continue;
            }
            Err(error) => {
                completeness = EnumerationCompleteness::Incomplete(IncompleteReason::Solver(
                    error.to_string(),
                ));
                continue;
            }
        };
        let trace_key = candidate
            .state
            .trace
            .iter()
            .map(|(id, outcome)| (id.clone(), outcome.clone()))
            .collect::<Vec<_>>();
        if !seen_traces.insert(trace_key) {
            return Err(symex_error(
                "enumeration produced a duplicate canonical selection trace",
            ));
        }
        let witness = build_witness(function, inputs, &model)?;
        paths.push(PathResult {
            condition: PathCondition::from_smtlib(condition),
            result: materialize_value(&arena, &candidate.value),
            trace: candidate.state.trace,
            witness,
        });
    }
    paths.sort_by_key(|path| {
        path.trace
            .iter()
            .map(|(id, outcome)| (id.clone(), outcome.clone()))
            .collect::<Vec<_>>()
    });
    if let Some(limit) = options.max_paths
        && paths.len() > limit
    {
        paths.truncate(limit);
        completeness = EnumerationCompleteness::Incomplete(IncompleteReason::PathLimit { limit });
    }
    Ok(EnumerationResult {
        parameters,
        paths,
        completeness,
        statistics: EnumerationStatistics {
            expression_nodes: arena.node_count(),
            evaluated_nodes,
            cache_hits,
            concrete_choices,
            symbolic_outcomes,
            solver_queries,
            infeasible_candidates,
            construction_time,
            solver_time,
        },
    })
}

fn lower_constraint(
    arena: &mut ExprArena,
    parameters: &[SymbolicParameter],
    constraint: &InputConstraint,
) -> Result<ExprId, XlsynthError> {
    match constraint {
        InputConstraint::Bool(value) => Ok(arena.bool_const(*value)),
        InputConstraint::Not(inner) => {
            let inner = lower_constraint(arena, parameters, inner)?;
            Ok(arena.bool_not(inner))
        }
        InputConstraint::And(constraints) => {
            let constraints = constraints
                .iter()
                .map(|constraint| lower_constraint(arena, parameters, constraint))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(arena.bool_and(constraints))
        }
        InputConstraint::Or(constraints) => {
            let constraints = constraints
                .iter()
                .map(|constraint| lower_constraint(arena, parameters, constraint))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(arena.bool_or(constraints))
        }
        InputConstraint::Compare {
            operation,
            lhs,
            rhs,
        } => {
            let lhs = lower_term(arena, parameters, lhs)?;
            let rhs = lower_term(arena, parameters, rhs)?;
            if arena.sort(lhs) != arena.sort(rhs) {
                return Err(symex_error("constraint comparison width mismatch"));
            }
            let comparison = match operation {
                ConstraintComparison::Equal | ConstraintComparison::NotEqual => CompareOp::Eq,
                ConstraintComparison::UnsignedLessThan => CompareOp::Ult,
                ConstraintComparison::UnsignedLessOrEqual => CompareOp::Ule,
                ConstraintComparison::SignedLessThan => CompareOp::Slt,
                ConstraintComparison::SignedLessOrEqual => CompareOp::Sle,
            };
            let result = arena.compare(comparison, lhs, rhs);
            if *operation == ConstraintComparison::NotEqual {
                Ok(arena.bool_not(result))
            } else {
                Ok(result)
            }
        }
    }
}

fn lower_term(
    arena: &mut ExprArena,
    parameters: &[SymbolicParameter],
    term: &ConstraintTerm,
) -> Result<ExprId, XlsynthError> {
    match term {
        ConstraintTerm::Input(name) => {
            let parameter = parameters
                .iter()
                .find(|parameter| parameter.name == *name)
                .ok_or_else(|| {
                    symex_error(format!("constraint names unknown input leaf {name:?}"))
                })?;
            Ok(arena.variable(name, Sort::Bits(parameter.bit_count)))
        }
        ConstraintTerm::Constant(value) => {
            let bits = value
                .to_bits()
                .map_err(|_| symex_error("constraint constants must be bits-typed XLS values"))?;
            if bits.get_bit_count() == 0 {
                return Err(symex_error(
                    "bits[0] constraint terms have no solver representation",
                ));
            }
            Ok(arena.bits_const(&bits))
        }
        ConstraintTerm::Not(inner) => {
            let inner = lower_term(arena, parameters, inner)?;
            Ok(arena.bit_unary(BitUnaryOp::Not, inner))
        }
        ConstraintTerm::Add(lhs, rhs)
        | ConstraintTerm::Sub(lhs, rhs)
        | ConstraintTerm::And(lhs, rhs)
        | ConstraintTerm::Or(lhs, rhs)
        | ConstraintTerm::Xor(lhs, rhs) => {
            let lhs_id = lower_term(arena, parameters, lhs)?;
            let rhs_id = lower_term(arena, parameters, rhs)?;
            if arena.sort(lhs_id) != arena.sort(rhs_id) {
                return Err(symex_error("constraint term width mismatch"));
            }
            let operation = match term {
                ConstraintTerm::Add(_, _) => BitBinaryOp::Add,
                ConstraintTerm::Sub(_, _) => BitBinaryOp::Sub,
                ConstraintTerm::And(_, _) => BitBinaryOp::And,
                ConstraintTerm::Or(_, _) => BitBinaryOp::Or,
                ConstraintTerm::Xor(_, _) => BitBinaryOp::Xor,
                _ => unreachable!(),
            };
            Ok(arena.bit_binary(operation, lhs_id, rhs_id))
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct NodeKey {
    function: String,
    node_index: usize,
    invocation: Vec<InvocationFrame>,
}

struct Frame<'a> {
    function: &'a Fn,
    arguments: Vec<Value>,
    invocation: Vec<InvocationFrame>,
}

#[derive(Clone)]
struct State {
    condition: ExprId,
    trace: BTreeMap<ChoiceId, ChoiceOutcome>,
    cache: HashMap<NodeKey, Value>,
}

struct Evaluated {
    state: State,
    value: Value,
}

struct PathEvaluator<'a> {
    package: &'a Package,
    arena: &'a mut ExprArena,
    syntactic_paths: usize,
    incomplete: Option<IncompleteReason>,
    evaluated_nodes: usize,
    cache_hits: usize,
    concrete_choices: usize,
    symbolic_outcomes: usize,
}

impl PathEvaluator<'_> {
    fn eval_node(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let key = NodeKey {
            function: frame.function.name.clone(),
            node_index: node_ref.index,
            invocation: frame.invocation.clone(),
        };
        if let Some(value) = state.cache.get(&key).cloned() {
            self.cache_hits += 1;
            return Ok(vec![Evaluated { state, value }]);
        }
        self.evaluated_nodes += 1;
        let node = frame.function.get_node(node_ref);
        let mut evaluated = match &node.payload {
            NodePayload::Nil => return Err(symex_error("the nil node cannot be evaluated")),
            NodePayload::GetParam(id) => {
                let index = frame
                    .function
                    .params
                    .iter()
                    .position(|parameter| parameter.id == *id)
                    .ok_or_else(|| symex_error("parameter id is absent from signature"))?;
                vec![Evaluated {
                    state,
                    value: frame.arguments[index].clone(),
                }]
            }
            NodePayload::Sel {
                selector,
                cases,
                default,
            } => self.eval_sel(frame, node_ref, *selector, cases, *default, state)?,
            NodePayload::PrioritySel {
                selector,
                cases,
                default,
            } => self.eval_priority_sel(frame, node_ref, *selector, cases, *default, state)?,
            NodePayload::OneHotSel { selector, cases } => {
                self.eval_one_hot_sel(frame, node_ref, *selector, cases, state)?
            }
            NodePayload::Invoke { to_apply, operands } => {
                self.eval_invoke(frame, node_ref, to_apply, operands, state)?
            }
            NodePayload::CountedFor {
                init,
                trip_count,
                stride,
                body,
                invariant_args,
            } => self.eval_counted_for(
                frame,
                node_ref,
                *init,
                *trip_count,
                *stride,
                body,
                invariant_args,
                state,
            )?,
            _ => {
                let operand_refs = operands(&node.payload);
                self.eval_operands(frame, &operand_refs, state)?
                    .into_iter()
                    .map(|(state, operands)| {
                        Ok(Evaluated {
                            state,
                            value: apply_pure_node(self.arena, node, &operands)?,
                        })
                    })
                    .collect::<Result<Vec<_>, XlsynthError>>()?
            }
        };
        for result in &mut evaluated {
            result.state.cache.insert(key.clone(), result.value.clone());
        }
        Ok(evaluated)
    }

    fn eval_operands(
        &mut self,
        frame: &Frame<'_>,
        operand_refs: &[NodeRef],
        state: State,
    ) -> Result<Vec<(State, Vec<Value>)>, XlsynthError> {
        let mut partials = vec![(state, Vec::with_capacity(operand_refs.len()))];
        for operand_ref in operand_refs {
            let mut next = Vec::new();
            for (state, values) in partials {
                for result in self.eval_node(frame, *operand_ref, state)? {
                    let mut result_values = values.clone();
                    result_values.push(result.value);
                    next.push((result.state, result_values));
                }
            }
            partials = next;
        }
        Ok(partials)
    }

    fn eval_sel(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        selector_ref: NodeRef,
        cases: &[NodeRef],
        default: Option<NodeRef>,
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        if cases.is_empty() {
            return Err(symex_error("sel has no cases"));
        }
        let mut results = Vec::new();
        for selected in self.eval_node(frame, selector_ref, state)? {
            let selector = as_bits(&selected.value, "sel selector")?;
            if let Some(concrete) = concrete_bits(self.arena, selector) {
                self.concrete_choices += 1;
                let index = bits_to_usize(&concrete);
                let outcome = index
                    .filter(|index| *index < cases.len())
                    .map_or(ChoiceOutcome::Default, ChoiceOutcome::Case);
                let case_ref = concrete_sel_case(&concrete, cases, default)?;
                let state =
                    self.record_choice(selected.state, self.choice_id(frame, node_ref), outcome)?;
                results.extend(self.eval_node(frame, case_ref, state)?);
                continue;
            }
            let outcomes = symbolic_sel_outcomes(self.arena, selector, cases, default)?;
            self.symbolic_outcomes += outcomes.len();
            for (outcome, guard, case_ref) in outcomes {
                let Some(state) = self.branch_state(
                    selected.state.clone(),
                    self.choice_id(frame, node_ref),
                    outcome,
                    guard,
                ) else {
                    continue;
                };
                results.extend(self.eval_node(frame, case_ref, state)?);
            }
        }
        Ok(results)
    }

    fn eval_priority_sel(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        selector_ref: NodeRef,
        cases: &[NodeRef],
        default: Option<NodeRef>,
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let default = default.ok_or_else(|| symex_error("priority_sel requires a default"))?;
        let mut results = Vec::new();
        for selected in self.eval_node(frame, selector_ref, state)? {
            let selector = as_bits(&selected.value, "priority_sel selector")?;
            if let Some(concrete) = concrete_bits(self.arena, selector) {
                self.concrete_choices += 1;
                let selected_case = cases
                    .iter()
                    .enumerate()
                    .find(|(index, _)| {
                        *index < concrete.get_bit_count() && concrete.get_bit(*index).unwrap()
                    })
                    .map(|(index, case_ref)| (index, *case_ref));
                let (outcome, case_ref) = selected_case
                    .map_or((ChoiceOutcome::Default, default), |(index, case_ref)| {
                        (ChoiceOutcome::Case(index), case_ref)
                    });
                let state =
                    self.record_choice(selected.state, self.choice_id(frame, node_ref), outcome)?;
                results.extend(self.eval_node(frame, case_ref, state)?);
                continue;
            }
            let outcomes = symbolic_priority_outcomes(self.arena, selector, cases, default)?;
            self.symbolic_outcomes += outcomes.len();
            for (outcome, guard, case_ref) in outcomes {
                let Some(state) = self.branch_state(
                    selected.state.clone(),
                    self.choice_id(frame, node_ref),
                    outcome,
                    guard,
                ) else {
                    continue;
                };
                results.extend(self.eval_node(frame, case_ref, state)?);
            }
        }
        Ok(results)
    }

    fn eval_one_hot_sel(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        selector_ref: NodeRef,
        cases: &[NodeRef],
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let result_type = &frame.function.get_node(node_ref).ty;
        let mut results = Vec::new();
        for selected in self.eval_node(frame, selector_ref, state)? {
            let selector = as_bits(&selected.value, "one_hot_sel selector")?;
            if let Some(concrete) = concrete_bits(self.arena, selector) {
                self.concrete_choices += 1;
                let mask = (0..cases.len())
                    .map(|index| {
                        index < concrete.get_bit_count() && concrete.get_bit(index).unwrap()
                    })
                    .collect::<Vec<_>>();
                let selected_cases = cases
                    .iter()
                    .zip(&mask)
                    .filter_map(|(case_ref, selected)| selected.then_some(*case_ref))
                    .collect::<Vec<_>>();
                let state = self.record_choice(
                    selected.state,
                    self.choice_id(frame, node_ref),
                    ChoiceOutcome::OneHotMask(mask),
                )?;
                results.extend(self.eval_deep_or_cases(
                    frame,
                    &selected_cases,
                    result_type,
                    state,
                )?);
                continue;
            }
            let relevant = selector.bit_count.min(cases.len());
            let combination_count = 1_usize.checked_shl(relevant as u32);
            let available = SYNTACTIC_PATH_SAFETY_LIMIT.saturating_sub(self.syntactic_paths);
            let count = combination_count.unwrap_or(usize::MAX).min(available);
            self.symbolic_outcomes += count;
            if combination_count.is_none_or(|total| total > available) && self.incomplete.is_none()
            {
                let choice = self.choice_id(frame, node_ref);
                self.incomplete = Some(IncompleteReason::ResourceLimit {
                    limit: SYNTACTIC_PATH_SAFETY_LIMIT,
                    choice,
                });
            }
            for mask in 0..count {
                let bits = (0..cases.len())
                    .map(|index| index < relevant && mask & (1_usize << index) != 0)
                    .collect::<Vec<_>>();
                let guard = one_hot_mask_guard(self.arena, selector, &bits[..relevant])?;
                let Some(state) = self.branch_state(
                    selected.state.clone(),
                    self.choice_id(frame, node_ref),
                    ChoiceOutcome::OneHotMask(bits.clone()),
                    guard,
                ) else {
                    continue;
                };
                let selected_cases = cases
                    .iter()
                    .zip(&bits)
                    .filter_map(|(case_ref, selected)| selected.then_some(*case_ref))
                    .collect::<Vec<_>>();
                results.extend(self.eval_deep_or_cases(
                    frame,
                    &selected_cases,
                    result_type,
                    state,
                )?);
            }
        }
        Ok(results)
    }

    fn eval_deep_or_cases(
        &mut self,
        frame: &Frame<'_>,
        cases: &[NodeRef],
        result_type: &Type,
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let zero = zero_value(self.arena, result_type)?;
        let mut partials = vec![Evaluated { state, value: zero }];
        for case_ref in cases {
            let mut next = Vec::new();
            for partial in partials {
                for case in self.eval_node(frame, *case_ref, partial.state)? {
                    next.push(Evaluated {
                        state: case.state,
                        value: deep_or(self.arena, &partial.value, &case.value)?,
                    });
                }
            }
            partials = next;
        }
        Ok(partials)
    }

    fn eval_invoke(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        callee_name: &str,
        operand_refs: &[NodeRef],
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let callee = self.package.get_fn(callee_name).ok_or_else(|| {
            symex_error(format!(
                "invoked function {callee_name:?} is absent from package"
            ))
        })?;
        let mut results = Vec::new();
        for (state, arguments) in self.eval_operands(frame, operand_refs, state)? {
            let mut invocation = frame.invocation.clone();
            invocation.push(InvocationFrame::Invoke {
                caller: frame.function.name.clone(),
                node_id: frame.function.get_node(node_ref).text_id,
            });
            let callee_frame = Frame {
                function: callee,
                arguments,
                invocation,
            };
            let ret = callee.ret_node_ref.ok_or_else(|| {
                symex_error(format!(
                    "invoked function {} has no return node",
                    callee.name
                ))
            })?;
            results.extend(self.eval_node(&callee_frame, ret, state)?);
        }
        Ok(results)
    }

    #[allow(clippy::too_many_arguments)]
    fn eval_counted_for(
        &mut self,
        frame: &Frame<'_>,
        node_ref: NodeRef,
        init: NodeRef,
        trip_count: usize,
        stride: usize,
        body_name: &str,
        invariant_args: &[NodeRef],
        state: State,
    ) -> Result<Vec<Evaluated>, XlsynthError> {
        let body = self.package.get_fn(body_name).ok_or_else(|| {
            symex_error(format!(
                "counted_for body {body_name:?} is absent from package"
            ))
        })?;
        let induction_width = match body.params.first().map(|parameter| &parameter.ty) {
            Some(Type::Bits(width)) => *width,
            _ => {
                return Err(symex_error(
                    "counted_for induction parameter must be bits-typed",
                ));
            }
        };
        let mut refs = vec![init];
        refs.extend_from_slice(invariant_args);
        let mut loop_states = self
            .eval_operands(frame, &refs, state)?
            .into_iter()
            .map(|(state, values)| LoopState {
                state,
                carry: values[0].clone(),
                invariants: values[1..].to_vec(),
            })
            .collect::<Vec<_>>();
        for iteration in 0..trip_count {
            let induction = iteration
                .checked_mul(stride)
                .ok_or_else(|| symex_error("counted_for induction value overflowed usize"))?;
            let induction = induction_value(self.arena, induction_width, induction)?;
            let mut next = Vec::new();
            for loop_state in loop_states {
                let mut arguments = Vec::with_capacity(2 + loop_state.invariants.len());
                arguments.push(induction.clone());
                arguments.push(loop_state.carry);
                arguments.extend(loop_state.invariants.iter().cloned());
                let mut invocation = frame.invocation.clone();
                invocation.push(InvocationFrame::CountedFor {
                    caller: frame.function.name.clone(),
                    node_id: frame.function.get_node(node_ref).text_id,
                    iteration,
                });
                let body_frame = Frame {
                    function: body,
                    arguments,
                    invocation,
                };
                let ret = body.ret_node_ref.ok_or_else(|| {
                    symex_error(format!("counted_for body {} has no return node", body.name))
                })?;
                for result in self.eval_node(&body_frame, ret, loop_state.state)? {
                    next.push(LoopState {
                        state: result.state,
                        carry: result.value,
                        invariants: loop_state.invariants.clone(),
                    });
                }
            }
            loop_states = next;
        }
        Ok(loop_states
            .into_iter()
            .map(|loop_state| Evaluated {
                state: loop_state.state,
                value: loop_state.carry,
            })
            .collect())
    }

    fn branch_state(
        &mut self,
        mut state: State,
        choice: ChoiceId,
        outcome: ChoiceOutcome,
        guard: ExprId,
    ) -> Option<State> {
        if self.syntactic_paths >= SYNTACTIC_PATH_SAFETY_LIMIT {
            if self.incomplete.is_none() {
                self.incomplete = Some(IncompleteReason::ResourceLimit {
                    limit: SYNTACTIC_PATH_SAFETY_LIMIT,
                    choice,
                });
            }
            return None;
        }
        if state.trace.insert(choice, outcome).is_some() {
            return None;
        }
        state.condition = self.arena.bool_and([state.condition, guard]);
        if self.arena.bool_value(state.condition) == Some(false) {
            return None;
        }
        self.syntactic_paths = self.syntactic_paths.saturating_add(1);
        Some(state)
    }

    fn record_choice(
        &self,
        mut state: State,
        choice: ChoiceId,
        outcome: ChoiceOutcome,
    ) -> Result<State, XlsynthError> {
        if state.trace.insert(choice.clone(), outcome).is_some() {
            return Err(symex_error(format!(
                "choice {choice:?} was evaluated more than once in one path"
            )));
        }
        Ok(state)
    }

    fn choice_id(&self, frame: &Frame<'_>, node_ref: NodeRef) -> ChoiceId {
        ChoiceId {
            function: frame.function.name.clone(),
            node_id: frame.function.get_node(node_ref).text_id,
            invocation: frame.invocation.clone(),
        }
    }
}

struct LoopState {
    state: State,
    carry: Value,
    invariants: Vec<Value>,
}

fn as_bits<'a>(value: &'a Value, description: &str) -> Result<&'a BitsValue, XlsynthError> {
    match value {
        Value::Bits(bits) => Ok(bits),
        Value::Tuple(_) | Value::Array(_) => {
            Err(symex_error(format!("{description} must be bits-typed")))
        }
    }
}

fn concrete_bits(arena: &ExprArena, value: &BitsValue) -> Option<IrBits> {
    if value.bit_count == 0 {
        Some(IrBits::zero(0))
    } else {
        value.expression.and_then(|id| arena.bits_value(id))
    }
}

fn concrete_sel_case(
    selector: &IrBits,
    cases: &[NodeRef],
    default: Option<NodeRef>,
) -> Result<NodeRef, XlsynthError> {
    let index = bits_to_usize(selector);
    if let Some(case) = index.and_then(|index| cases.get(index)).copied() {
        Ok(case)
    } else if let Some(default) = default {
        Ok(default)
    } else {
        cases
            .last()
            .copied()
            .ok_or_else(|| symex_error("sel has no cases or default"))
    }
}

fn symbolic_sel_outcomes(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[NodeRef],
    default: Option<NodeRef>,
) -> Result<Vec<(ChoiceOutcome, ExprId, NodeRef)>, XlsynthError> {
    let mut equalities = Vec::with_capacity(cases.len());
    let mut outcomes = Vec::with_capacity(cases.len() + 1);
    for (index, case_ref) in cases.iter().copied().enumerate() {
        let guard = equality_guard(arena, selector, index)?;
        equalities.push(guard);
        outcomes.push((ChoiceOutcome::Case(index), guard, case_ref));
    }
    let fallback_guard = if equalities.is_empty() {
        arena.bool_const(true)
    } else {
        let any_explicit = arena.bool_or(equalities);
        arena.bool_not(any_explicit)
    };
    let fallback_value = default.unwrap_or(
        *cases
            .last()
            .ok_or_else(|| symex_error("sel has no cases"))?,
    );
    outcomes.push((ChoiceOutcome::Default, fallback_guard, fallback_value));
    Ok(outcomes)
}

fn symbolic_priority_outcomes(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[NodeRef],
    default: NodeRef,
) -> Result<Vec<(ChoiceOutcome, ExprId, NodeRef)>, XlsynthError> {
    let relevant = selector.bit_count.min(cases.len());
    let mut prior_clear = arena.bool_const(true);
    let mut outcomes = Vec::with_capacity(relevant + 1);
    for (index, case_ref) in cases.iter().copied().take(relevant).enumerate() {
        let bit = arena.extract(bits_expr(selector)?, index, 1);
        let set = arena.bit_is_one(bit);
        let guard = arena.bool_and([prior_clear, set]);
        outcomes.push((ChoiceOutcome::Case(index), guard, case_ref));
        let clear = arena.bool_not(set);
        prior_clear = arena.bool_and([prior_clear, clear]);
    }
    outcomes.push((ChoiceOutcome::Default, prior_clear, default));
    Ok(outcomes)
}

fn one_hot_mask_guard(
    arena: &mut ExprArena,
    selector: &BitsValue,
    mask: &[bool],
) -> Result<ExprId, XlsynthError> {
    let mut guards = Vec::with_capacity(mask.len());
    for (index, selected) in mask.iter().copied().enumerate() {
        let bit = arena.extract(bits_expr(selector)?, index, 1);
        let set = arena.bit_is_one(bit);
        guards.push(if selected { set } else { arena.bool_not(set) });
    }
    Ok(arena.bool_and(guards))
}

fn equality_guard(
    arena: &mut ExprArena,
    selector: &BitsValue,
    index: usize,
) -> Result<ExprId, XlsynthError> {
    selector_equals(arena, selector, index)
}

fn bits_to_usize(bits: &IrBits) -> Option<usize> {
    let mut result = 0_usize;
    for index in 0..bits.get_bit_count() {
        if !bits.get_bit(index).unwrap() {
            continue;
        }
        if index >= usize::BITS as usize {
            return None;
        }
        result |= 1_usize << index;
    }
    Some(result)
}

fn induction_value(
    arena: &mut ExprArena,
    width: usize,
    value: usize,
) -> Result<Value, XlsynthError> {
    if width == 0 {
        return Ok(Value::Bits(bits(0, None)?));
    }
    let bits_value = (0..width)
        .map(|index| index < usize::BITS as usize && value & (1_usize << index) != 0)
        .collect::<Vec<_>>();
    let expression = arena.bits_const(&IrBits::from_lsb_is_0(&bits_value));
    Ok(Value::Bits(bits(width, Some(expression))?))
}

fn build_witness(
    function: &Fn,
    inputs: Option<&[EvaluationInput]>,
    model: &BTreeMap<String, IrBits>,
) -> Result<PathWitness, XlsynthError> {
    let values = function
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            witness_value(
                &parameter.ty,
                inputs.map(|inputs| &inputs[index]),
                &format!("symex_arg_{index}"),
                model,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let symbolic_leaves = model
        .iter()
        .map(|(name, bits)| (name.clone(), IrValue::from_bits(bits)))
        .collect();
    Ok(PathWitness {
        inputs: values,
        symbolic_leaves,
    })
}

fn witness_value(
    ty: &Type,
    input: Option<&EvaluationInput>,
    name: &str,
    model: &BTreeMap<String, IrBits>,
) -> Result<IrValue, XlsynthError> {
    if let Some(EvaluationInput::Concrete(value)) = input {
        return Ok(value.clone());
    }
    match (ty, input) {
        (Type::Bits(0), _) => Ok(IrValue::from_bits(&IrBits::zero(0))),
        (Type::Bits(_), None | Some(EvaluationInput::Symbolic)) => model
            .get(name)
            .map(IrValue::from_bits)
            .ok_or_else(|| symex_error(format!("solver model omitted {name}"))),
        (Type::Tuple(types), None | Some(EvaluationInput::Symbolic)) => {
            let values = types
                .iter()
                .enumerate()
                .map(|(index, ty)| witness_value(ty, None, &format!("{name}_{index}"), model))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IrValue::make_tuple(&values))
        }
        (Type::Tuple(types), Some(EvaluationInput::Tuple(inputs))) => {
            let values = types
                .iter()
                .zip(inputs)
                .enumerate()
                .map(|(index, (ty, input))| {
                    witness_value(ty, Some(input), &format!("{name}_{index}"), model)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(IrValue::make_tuple(&values))
        }
        (Type::Array(array), None | Some(EvaluationInput::Symbolic)) => {
            let values = (0..array.element_count)
                .map(|index| {
                    witness_value(&array.element_type, None, &format!("{name}_{index}"), model)
                })
                .collect::<Result<Vec<_>, _>>()?;
            IrValue::make_array(&values)
        }
        (Type::Array(array), Some(EvaluationInput::Array(inputs))) => {
            let values = inputs
                .iter()
                .enumerate()
                .map(|(index, input)| {
                    witness_value(
                        &array.element_type,
                        Some(input),
                        &format!("{name}_{index}"),
                        model,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            IrValue::make_array(&values)
        }
        (Type::Token, _) => Err(symex_error("token witness is outside the pure value scope")),
        (ty, Some(input)) => Err(symex_error(format!(
            "input shape {input:?} does not match witness type {ty}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_nested_selects_with_canonical_inactivity() {
        let ir = r#"package test

top fn nested(x: bits[1], y: bits[1], a: bits[8], b: bits[8], c: bits[8]) -> bits[8] {
  inner: bits[8] = sel(y, cases=[a, b], id=6)
  ret outer: bits[8] = sel(x, cases=[c, inner], id=7)
}
"#;
        let result =
            enumerate_package_text(ir, "nested", None, &EnumerationOptions::default()).unwrap();
        assert_eq!(result.completeness, EnumerationCompleteness::Complete);
        assert_eq!(result.paths.len(), 3);
        let mut trace_lengths = result
            .paths
            .iter()
            .map(|path| path.trace.len())
            .collect::<Vec<_>>();
        trace_lengths.sort_unstable();
        assert_eq!(trace_lengths, vec![1, 2, 2]);
    }
}
