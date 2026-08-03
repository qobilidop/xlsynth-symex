// SPDX-License-Identifier: Apache-2.0

//! Persistent upstream-solver session used by selection enumeration.
//!
//! The symbolic arena remains backend-neutral. This module lowers it once into
//! the solver terms maintained by `xlsynth-prover`, then checks every candidate
//! guard incrementally with a balanced push/pop scope.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use xlsynth::{IrBits, XlsynthError};
use xlsynth_pir::ir::Type;
use xlsynth_prover::solver::easy_smt::{EasySmtConfig, EasySmtSolver};
use xlsynth_prover::solver::{BitVec, Response, Solver};

use crate::SymbolicParameter;
use crate::expr::{BitBinaryOp, BitUnaryOp, CompareOp, ExprArena, ExprId, ExprKind, Sort};

type EasySmtTerm = <EasySmtSolver as Solver>::Term;

pub(crate) enum Satisfiability {
    Sat(BTreeMap<String, IrBits>),
    Unsat,
    Indeterminate(String),
}

/// One persistent solver and the complete lowering of an expression arena.
pub(crate) struct SolverSession<'a> {
    solver: EasySmtSolver,
    terms: HashMap<ExprId, BitVec<EasySmtTerm>>,
    parameter_terms: BTreeMap<String, BitVec<EasySmtTerm>>,
    parameters: &'a [SymbolicParameter],
}

impl<'a> SolverSession<'a> {
    /// Starts Z3 once and lowers every arena node into the upstream solver API.
    pub(crate) fn new(
        arena: &ExprArena,
        parameters: &'a [SymbolicParameter],
        timeout: Duration,
    ) -> Result<Self, XlsynthError> {
        let timeout_ms = u64::try_from(timeout.as_millis())
            .unwrap_or(u64::MAX)
            .max(1);
        let mut config = EasySmtConfig::z3();
        // Z3 documents `-t` as a per-query soft timeout, so it remains valid
        // across the incremental checks performed by this persistent process.
        config.solver_args.push(format!("-t:{timeout_ms}"));
        let mut solver = EasySmtSolver::new(&config)
            .map_err(|error| solver_error(format!("failed to start z3: {error}")))?;
        let mut terms = HashMap::with_capacity(arena.node_count());
        let mut variables = HashMap::new();

        for (id, sort, kind) in arena.nodes() {
            let term = lower_node(&mut solver, arena, &terms, id, sort, kind)?;
            if let ExprKind::Variable(name) = kind
                && variables.insert(name.clone(), term.clone()).is_some()
            {
                return Err(solver_error(format!(
                    "arena declares variable {name:?} more than once"
                )));
            }
            terms.insert(id, term);
        }

        let parameter_terms = parameters
            .iter()
            .map(|parameter| {
                let term = variables.get(&parameter.name).cloned().ok_or_else(|| {
                    solver_error(format!(
                        "symbolic parameter {:?} has no arena variable",
                        parameter.name
                    ))
                })?;
                if term.get_width() != parameter.bit_count {
                    return Err(solver_error(format!(
                        "symbolic parameter {:?} has solver width {}, expected {}",
                        parameter.name,
                        term.get_width(),
                        parameter.bit_count
                    )));
                }
                Ok((parameter.name.clone(), term))
            })
            .collect::<Result<_, XlsynthError>>()?;

        Ok(Self {
            solver,
            terms,
            parameter_terms,
            parameters,
        })
    }

    /// Checks one arena guard and returns a complete arbitrary-width model.
    pub(crate) fn solve(&mut self, guard: ExprId) -> Result<Satisfiability, XlsynthError> {
        let guard = self
            .terms
            .get(&guard)
            .cloned()
            .ok_or_else(|| solver_error("guard is absent from the lowered arena"))?;
        if guard.get_width() != 1 {
            return Err(solver_error("guard did not lower to bits[1]"));
        }

        self.solver
            .push()
            .map_err(|error| solver_error(format!("failed to push solver scope: {error}")))?;
        let result = self.solve_in_scope(&guard);
        let popped = self
            .solver
            .pop()
            .map_err(|error| solver_error(format!("failed to pop solver scope: {error}")));
        match (result, popped) {
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), Ok(())) | (_, Err(error)) => Err(error),
        }
    }

    fn solve_in_scope(
        &mut self,
        guard: &BitVec<EasySmtTerm>,
    ) -> Result<Satisfiability, XlsynthError> {
        self.solver
            .assert(guard)
            .map_err(|error| solver_error(format!("failed to assert guard: {error}")))?;
        match self
            .solver
            .check()
            .map_err(|error| solver_error(format!("failed to check guard: {error}")))?
        {
            Response::Unsat => Ok(Satisfiability::Unsat),
            Response::Unknown => Ok(Satisfiability::Indeterminate(
                "solver returned unknown or reached its per-query timeout".to_owned(),
            )),
            Response::Sat => {
                let mut model = BTreeMap::new();
                for parameter in self.parameters {
                    let term = self
                        .parameter_terms
                        .get(&parameter.name)
                        .expect("validated parameter term must be present");
                    let value = self
                        .solver
                        .get_value(term, &Type::Bits(parameter.bit_count))
                        .map_err(|error| {
                            solver_error(format!(
                                "failed to read model value for {:?}: {error}",
                                parameter.name
                            ))
                        })?;
                    let bits = value.to_bits().map_err(|error| {
                        solver_error(format!(
                            "model value for {:?} is not bits-typed: {error}",
                            parameter.name
                        ))
                    })?;
                    model.insert(parameter.name.clone(), bits);
                }
                Ok(Satisfiability::Sat(model))
            }
        }
    }
}

fn lower_node(
    solver: &mut EasySmtSolver,
    arena: &ExprArena,
    terms: &HashMap<ExprId, BitVec<EasySmtTerm>>,
    id: ExprId,
    sort: Sort,
    kind: &ExprKind,
) -> Result<BitVec<EasySmtTerm>, XlsynthError> {
    let term = |id: ExprId| {
        terms
            .get(&id)
            .cloned()
            .ok_or_else(|| solver_error("arena node is not in dependency order"))
    };
    let result = match kind {
        ExprKind::BoolConst(true) => solver.true_bv(),
        ExprKind::BoolConst(false) => solver.false_bv(),
        ExprKind::BitsConst(_) => {
            let Sort::Bits(width) = sort else {
                return Err(solver_error("bit-vector constant has Boolean sort"));
            };
            solver.from_raw_str(width, &arena.to_smtlib(id))
        }
        ExprKind::Variable(name) => {
            let width = match sort {
                Sort::Bool => 1,
                Sort::Bits(width) => width,
            };
            solver
                .declare(name, width)
                .map_err(|error| solver_error(format!("failed to declare {name:?}: {error}")))?
        }
        ExprKind::BoolNot(arg) => solver.not(&term(*arg)?),
        ExprKind::BoolAnd(args) => {
            let args = args
                .iter()
                .map(|arg| term(*arg))
                .collect::<Result<Vec<_>, _>>()?;
            fold_terms(solver, &args, Solver::and)?
        }
        ExprKind::BoolOr(args) => {
            let args = args
                .iter()
                .map(|arg| term(*arg))
                .collect::<Result<Vec<_>, _>>()?;
            fold_terms(solver, &args, Solver::or)?
        }
        ExprKind::BitUnary(op, arg) => match op {
            BitUnaryOp::Not => solver.not(&term(*arg)?),
            BitUnaryOp::Neg => solver.neg(&term(*arg)?),
        },
        ExprKind::BitBinary(op, lhs, rhs) => {
            let lhs = term(*lhs)?;
            let rhs = term(*rhs)?;
            match op {
                BitBinaryOp::Add => solver.add(&lhs, &rhs),
                BitBinaryOp::Sub => solver.sub(&lhs, &rhs),
                BitBinaryOp::Mul => solver.mul(&lhs, &rhs),
                BitBinaryOp::Udiv => solver.udiv(&lhs, &rhs),
                BitBinaryOp::Sdiv => solver.sdiv(&lhs, &rhs),
                BitBinaryOp::Urem => solver.urem(&lhs, &rhs),
                BitBinaryOp::Srem => solver.srem(&lhs, &rhs),
                BitBinaryOp::And => solver.and(&lhs, &rhs),
                BitBinaryOp::Or => solver.or(&lhs, &rhs),
                BitBinaryOp::Xor => solver.xor(&lhs, &rhs),
                BitBinaryOp::Shl => solver.shl(&lhs, &rhs),
                BitBinaryOp::Lshr => solver.lshr(&lhs, &rhs),
                BitBinaryOp::Ashr => solver.ashr(&lhs, &rhs),
            }
        }
        ExprKind::Compare(op, lhs, rhs) => {
            let lhs = term(*lhs)?;
            let rhs = term(*rhs)?;
            match op {
                CompareOp::Eq => solver.eq(&lhs, &rhs),
                CompareOp::Ult => solver.ult(&lhs, &rhs),
                CompareOp::Ule => solver.ule(&lhs, &rhs),
                CompareOp::Ugt => solver.ugt(&lhs, &rhs),
                CompareOp::Uge => solver.uge(&lhs, &rhs),
                CompareOp::Slt => solver.slt(&lhs, &rhs),
                CompareOp::Sle => solver.sle(&lhs, &rhs),
                CompareOp::Sgt => solver.sgt(&lhs, &rhs),
                CompareOp::Sge => solver.sge(&lhs, &rhs),
            }
        }
        ExprKind::Ite(condition, then_id, else_id) => {
            solver.ite(&term(*condition)?, &term(*then_id)?, &term(*else_id)?)
        }
        ExprKind::Concat(args) => {
            let args = args
                .iter()
                .map(|arg| term(*arg))
                .collect::<Result<Vec<_>, _>>()?;
            fold_terms(solver, &args, Solver::concat)?
        }
        ExprKind::Extract { arg, start, width } => solver.slice(&term(*arg)?, *start, *width),
        ExprKind::Extend {
            arg,
            signed,
            amount,
        } => solver.extend(&term(*arg)?, *amount, *signed),
    };
    let expected_width = match sort {
        Sort::Bool => 1,
        Sort::Bits(width) => width,
    };
    if result.get_width() != expected_width {
        return Err(solver_error(format!(
            "lowered arena node has width {}, expected {expected_width}",
            result.get_width()
        )));
    }
    Ok(result)
}

fn fold_terms(
    solver: &mut EasySmtSolver,
    terms: &[BitVec<EasySmtTerm>],
    operation: fn(
        &mut EasySmtSolver,
        &BitVec<EasySmtTerm>,
        &BitVec<EasySmtTerm>,
    ) -> BitVec<EasySmtTerm>,
) -> Result<BitVec<EasySmtTerm>, XlsynthError> {
    let (first, rest) = terms
        .split_first()
        .ok_or_else(|| solver_error("n-ary arena expression has no operands"))?;
    Ok(rest.iter().fold(first.clone(), |result, term| {
        operation(solver, &result, term)
    }))
}

fn solver_error(message: impl Into<String>) -> XlsynthError {
    XlsynthError(format!("xlsynth-symex solver: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InputLeaf;
    use xlsynth::IrValue;

    #[test]
    fn solves_wide_models_and_reuses_the_session() {
        let mut arena = ExprArena::default();
        let x = arena.variable("x", Sort::Bits(5));
        let wide = arena.variable("wide", Sort::Bits(80));
        let x_value = arena.bits_const_u64(5, 21);
        let wide_bits = IrValue::parse_typed("bits[80]:0x1234_5678_9abc_def0_1234")
            .unwrap()
            .to_bits()
            .unwrap();
        let wide_value = arena.bits_const(&wide_bits);
        let x_equal = arena.compare(CompareOp::Eq, x, x_value);
        let wide_equal = arena.compare(CompareOp::Eq, wide, wide_value);
        let sat_guard = arena.bool_and([x_equal, wide_equal]);
        let not_x_equal = arena.bool_not(x_equal);
        let unsat_guard = arena.bool_and([x_equal, not_x_equal]);
        let parameters = vec![
            SymbolicParameter {
                input: InputLeaf::argument(0),
                name: "x".to_owned(),
                bit_count: 5,
            },
            SymbolicParameter {
                input: InputLeaf::argument(1),
                name: "wide".to_owned(),
                bit_count: 80,
            },
        ];
        let mut session = SolverSession::new(&arena, &parameters, Duration::from_secs(10)).unwrap();

        let Satisfiability::Sat(model) = session.solve(sat_guard).unwrap() else {
            panic!("constraint must be satisfiable");
        };
        assert_eq!(model["x"].to_u64().unwrap(), 21);
        assert_eq!(
            model["wide"].to_string(),
            "bits[80]:0x1234_5678_9abc_def0_1234"
        );
        assert!(matches!(
            session.solve(unsat_guard).unwrap(),
            Satisfiability::Unsat
        ));
    }
}
