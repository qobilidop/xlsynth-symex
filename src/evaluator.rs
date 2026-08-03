// SPDX-License-Identifier: Apache-2.0

use xlsynth::{IrValue, XlsynthError};
use xlsynth_pir::ir::{
    Binop, Fn, NaryOp, Node, NodePayload, NodeRef, Package, PackageMember, Type, Unop,
};
use xlsynth_pir::ir_parser::Parser;

use crate::expr::{BitBinaryOp, BitUnaryOp, CompareOp, ExprArena, ExprId, Sort};
use crate::{
    EvaluationInput, PathCondition, SymbolicBits, SymbolicParameter, SymbolicValue, SymexResult,
};

#[derive(Clone, Debug)]
pub(crate) struct BitsValue {
    pub(crate) bit_count: usize,
    pub(crate) expression: Option<ExprId>,
}

#[derive(Clone, Debug)]
pub(crate) enum Value {
    Bits(BitsValue),
    Tuple(Vec<Value>),
    Array(Vec<Value>),
}

pub(crate) fn evaluate_function_text(function_text: &str) -> Result<SymexResult, XlsynthError> {
    evaluate_function_text_with_optional_inputs(function_text, None)
}

pub(crate) fn evaluate_function_text_with_inputs(
    function_text: &str,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluate_function_text_with_optional_inputs(function_text, Some(inputs))
}

fn evaluate_function_text_with_optional_inputs(
    function_text: &str,
    inputs: Option<&[EvaluationInput]>,
) -> Result<SymexResult, XlsynthError> {
    let package_text = format!("package standalone\n\n{function_text}");
    let mut package = Parser::new(&package_text)
        .parse_and_validate_package()
        .map_err(|error| symex_error(format!("failed to parse XLS IR function: {error}")))?;
    xlsynth_pir::desugar_extensions::desugar_extensions_in_package(&mut package)
        .map_err(|error| symex_error(format!("failed to desugar extension operations: {error}")))?;
    normalize_functions(&mut package);
    let function_name = package
        .get_top_fn()
        .ok_or_else(|| symex_error("standalone IR has no function"))?
        .name
        .clone();
    evaluate_parsed(&package, &function_name, inputs)
}

pub(crate) fn evaluate_package_text(
    package_text: &str,
    function_name: &str,
) -> Result<SymexResult, XlsynthError> {
    evaluate_package_text_with_optional_inputs(package_text, function_name, None)
}

pub(crate) fn evaluate_package_text_with_inputs(
    package_text: &str,
    function_name: &str,
    inputs: &[EvaluationInput],
) -> Result<SymexResult, XlsynthError> {
    evaluate_package_text_with_optional_inputs(package_text, function_name, Some(inputs))
}

fn evaluate_package_text_with_optional_inputs(
    package_text: &str,
    function_name: &str,
    inputs: Option<&[EvaluationInput]>,
) -> Result<SymexResult, XlsynthError> {
    let mut package = Parser::new(package_text)
        .parse_and_validate_package()
        .map_err(|error| symex_error(format!("failed to parse XLS IR package: {error}")))?;
    xlsynth_pir::desugar_extensions::desugar_extensions_in_package(&mut package)
        .map_err(|error| symex_error(format!("failed to desugar extension operations: {error}")))?;
    normalize_functions(&mut package);
    evaluate_parsed(&package, function_name, inputs)
}

fn normalize_functions(package: &mut Package) {
    for member in &mut package.members {
        let function = match member {
            PackageMember::Function(function) | PackageMember::Block { func: function, .. } => {
                function
            }
        };
        *function = xlsynth_pir::dce::remove_dead_nodes(function);
    }
}

fn evaluate_parsed(
    package: &Package,
    function_name: &str,
    inputs: Option<&[EvaluationInput]>,
) -> Result<SymexResult, XlsynthError> {
    let function = package
        .get_fn(function_name)
        .ok_or_else(|| symex_error(format!("function {function_name:?} is absent from package")))?;
    let mut arena = ExprArena::default();
    let mut parameters = Vec::new();
    if let Some(inputs) = inputs
        && inputs.len() != function.params.len()
    {
        return Err(symex_error(format!(
            "function {function_name:?} expects {} inputs, got {}",
            function.params.len(),
            inputs.len()
        )));
    }
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
    let result = Evaluator {
        package,
        arena: &mut arena,
    }
    .evaluate_function(function, arguments)?;
    let result = materialize_value(&arena, &result);
    let mut result_smtlib = parameters
        .iter()
        .map(|parameter| {
            format!(
                "(declare-const {} (_ BitVec {}))\n",
                parameter.name, parameter.bit_count
            )
        })
        .collect::<String>();
    if let Some(bits) = result.as_bits()
        && bits.bit_count > 0
    {
        result_smtlib.push_str(&format!(
            "(define-fun xlsynth_symex_result () (_ BitVec {}) {})\n",
            bits.bit_count, bits.expression
        ));
    }
    Ok(SymexResult {
        path_condition: PathCondition::default(),
        parameters,
        result,
        result_smtlib,
    })
}

struct Evaluator<'a> {
    package: &'a Package,
    arena: &'a mut ExprArena,
}

impl Evaluator<'_> {
    fn evaluate_function(
        &mut self,
        function: &Fn,
        arguments: Vec<Value>,
    ) -> Result<Value, XlsynthError> {
        if arguments.len() != function.params.len() {
            return Err(symex_error(format!(
                "function {} expects {} arguments, got {}",
                function.name,
                function.params.len(),
                arguments.len()
            )));
        }
        let mut values = vec![None; function.nodes.len()];
        for node_ref in function.node_refs() {
            let node = function.get_node(node_ref);
            let value = match &node.payload {
                NodePayload::Nil => continue,
                NodePayload::GetParam(id) => arguments
                    .get(
                        function
                            .params
                            .iter()
                            .position(|parameter| parameter.id == *id)
                            .ok_or_else(|| symex_error("parameter id is absent from signature"))?,
                    )
                    .cloned()
                    .ok_or_else(|| symex_error("parameter id is out of bounds"))?,
                NodePayload::Literal(value) => {
                    literal_value(self.arena, &node.ty, value).map_err(|error| {
                        symex_error(format!("literal at node {}: {error}", node.text_id))
                    })?
                }
                NodePayload::Tuple(elements) => Value::Tuple(
                    elements
                        .iter()
                        .map(|element| get_value(&values, *element).cloned())
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                NodePayload::Array(elements) => Value::Array(
                    elements
                        .iter()
                        .map(|element| get_value(&values, *element).cloned())
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                NodePayload::ArrayIndex { array, indices, .. } => {
                    let _ = (array, indices);
                    apply_from_environment(self.arena, node, &values)?
                }
                NodePayload::ArrayUpdate {
                    array,
                    value,
                    indices,
                    ..
                } => {
                    let _ = (array, value, indices);
                    apply_from_environment(self.arena, node, &values)?
                }
                NodePayload::TupleIndex { tuple, index } => match get_value(&values, *tuple)? {
                    Value::Tuple(elements) => elements.get(*index).cloned().ok_or_else(|| {
                        symex_error(format!("tuple index {index} is out of bounds"))
                    })?,
                    Value::Bits(_) | Value::Array(_) => {
                        return Err(symex_error("tuple_index operand is not tuple-typed"));
                    }
                },
                NodePayload::Binop(Binop::Gate | Binop::Smulp | Binop::Umulp, _, _) => {
                    apply_from_environment(self.arena, node, &values)?
                }
                NodePayload::Binop(op, lhs, rhs) => {
                    let lhs = get_bits(&values, *lhs)?;
                    let rhs = get_bits(&values, *rhs)?;
                    Value::Bits(evaluate_binop(
                        self.arena,
                        *op,
                        lhs,
                        rhs,
                        bits_width(&node.ty)?,
                    )?)
                }
                NodePayload::Unop(Unop::Identity, arg) => get_value(&values, *arg)?.clone(),
                NodePayload::Unop(op, arg) => Value::Bits(evaluate_unop(
                    self.arena,
                    *op,
                    get_bits(&values, *arg)?,
                    bits_width(&node.ty)?,
                )?),
                NodePayload::Nary(op, operands) => {
                    let operands = operands
                        .iter()
                        .map(|operand| get_bits(&values, *operand))
                        .collect::<Result<Vec<_>, _>>()?;
                    Value::Bits(evaluate_nary(
                        self.arena,
                        *op,
                        &operands,
                        bits_width(&node.ty)?,
                    )?)
                }
                NodePayload::ZeroExt { arg, new_bit_count } => Value::Bits(resize(
                    self.arena,
                    get_bits(&values, *arg)?,
                    *new_bit_count,
                    false,
                )?),
                NodePayload::SignExt { arg, new_bit_count } => Value::Bits(resize(
                    self.arena,
                    get_bits(&values, *arg)?,
                    *new_bit_count,
                    true,
                )?),
                NodePayload::BitSlice { arg, start, width } => {
                    let arg = get_bits(&values, *arg)?;
                    let expression = if *width == 0 {
                        None
                    } else {
                        Some(self.arena.extract(bits_expr(arg)?, *start, *width))
                    };
                    Value::Bits(bits(*width, expression)?)
                }
                NodePayload::DynamicBitSlice { arg, start, width } => {
                    let _ = (arg, start, width);
                    apply_from_environment(self.arena, node, &values)?
                }
                NodePayload::OneHot { arg, lsb_prio } => Value::Bits(evaluate_one_hot(
                    self.arena,
                    get_bits(&values, *arg)?,
                    *lsb_prio,
                )?),
                NodePayload::Encode { arg } => Value::Bits(evaluate_encode(
                    self.arena,
                    get_bits(&values, *arg)?,
                    bits_width(&node.ty)?,
                )?),
                NodePayload::Sel {
                    selector,
                    cases,
                    default,
                } => {
                    let selector = get_bits(&values, *selector)?;
                    let cases = cases
                        .iter()
                        .map(|case| get_value(&values, *case).cloned())
                        .collect::<Result<Vec<_>, _>>()?;
                    let default = default
                        .map(|value| get_value(&values, value).cloned())
                        .transpose()?;
                    select_structural(self.arena, selector, &cases, default.as_ref())?
                }
                NodePayload::Invoke { to_apply, operands } => {
                    let callee = self.package.get_fn(to_apply).ok_or_else(|| {
                        symex_error(format!(
                            "invoked function {to_apply:?} is absent from package"
                        ))
                    })?;
                    let arguments = operands
                        .iter()
                        .map(|operand| get_value(&values, *operand).cloned())
                        .collect::<Result<Vec<_>, _>>()?;
                    self.evaluate_function(callee, arguments)?
                }
                NodePayload::CountedFor {
                    init,
                    trip_count,
                    stride,
                    body,
                    invariant_args,
                } => {
                    let body_function = self.package.get_fn(body).ok_or_else(|| {
                        symex_error(format!("counted_for body {body:?} is absent from package"))
                    })?;
                    let induction_width = body_function
                        .params
                        .first()
                        .ok_or_else(|| symex_error("counted_for body has no induction parameter"))
                        .and_then(|parameter| bits_width(&parameter.ty))?;
                    let invariants = invariant_args
                        .iter()
                        .map(|operand| get_value(&values, *operand).cloned())
                        .collect::<Result<Vec<_>, _>>()?;
                    let mut carry = get_value(&values, *init)?.clone();
                    for iteration in 0..*trip_count {
                        let induction = iteration.checked_mul(*stride).ok_or_else(|| {
                            symex_error("counted_for induction value overflowed usize")
                        })?;
                        let expression = if induction_width == 0 {
                            None
                        } else {
                            Some(self.arena.bits_const_u64(induction_width, induction as u64))
                        };
                        let mut arguments = Vec::with_capacity(2 + invariants.len());
                        arguments.push(Value::Bits(bits(induction_width, expression)?));
                        arguments.push(carry);
                        arguments.extend(invariants.iter().cloned());
                        carry = self.evaluate_function(body_function, arguments)?;
                    }
                    carry
                }
                payload => {
                    let _ = payload;
                    apply_from_environment(self.arena, node, &values)?
                }
            };
            values[node_ref.index] = Some(value);
        }
        let ret = function
            .ret_node_ref
            .ok_or_else(|| symex_error(format!("function {} has no return node", function.name)))?;
        get_value(&values, ret).cloned()
    }
}

fn apply_from_environment(
    arena: &mut ExprArena,
    node: &Node,
    values: &[Option<Value>],
) -> Result<Value, XlsynthError> {
    let operands = xlsynth_pir::ir_utils::operands(&node.payload)
        .into_iter()
        .map(|operand| get_value(values, operand).cloned())
        .collect::<Result<Vec<_>, _>>()?;
    apply_pure_node(arena, node, &operands)
}

pub(crate) fn apply_pure_node(
    arena: &mut ExprArena,
    node: &Node,
    operands: &[Value],
) -> Result<Value, XlsynthError> {
    let value = match &node.payload {
        NodePayload::Literal(value) => literal_value(arena, &node.ty, value)?,
        NodePayload::Tuple(_) => Value::Tuple(operands.to_vec()),
        NodePayload::Array(_) => Value::Array(operands.to_vec()),
        NodePayload::ArrayConcat(_) => {
            let mut result = Vec::new();
            for operand in operands {
                let Value::Array(elements) = operand else {
                    return Err(symex_error("array_concat operand is not array-typed"));
                };
                result.extend(elements.iter().cloned());
            }
            Value::Array(result)
        }
        NodePayload::ArraySlice { width, .. } => {
            let [Value::Array(elements), Value::Bits(start)] = operands else {
                return Err(symex_error("array_slice operand types are invalid"));
            };
            let last = elements
                .last()
                .ok_or_else(|| symex_error("array_slice does not support an empty array"))?;
            let mut result = Vec::with_capacity(*width);
            for offset in 0..*width {
                let cases = (0..elements.len())
                    .map(|start| {
                        elements[start.saturating_add(offset).min(elements.len() - 1)].clone()
                    })
                    .collect::<Vec<_>>();
                result.push(select_structural(arena, start, &cases, Some(last))?);
            }
            Value::Array(result)
        }
        NodePayload::TupleIndex { index, .. } => {
            let [Value::Tuple(elements)] = operands else {
                return Err(symex_error("tuple_index operand is not tuple-typed"));
            };
            elements
                .get(*index)
                .cloned()
                .ok_or_else(|| symex_error(format!("tuple index {index} is out of bounds")))?
        }
        NodePayload::Binop(Binop::Gate, _, _) => {
            let [Value::Bits(predicate), gated] = operands else {
                return Err(symex_error("gate predicate must be bits-typed"));
            };
            if predicate.bit_count != 1 {
                return Err(symex_error("gate predicate must be bits[1]"));
            }
            let condition = arena.bit_is_one(bits_expr(predicate)?);
            let zero = zero_value(arena, &node.ty)?;
            select_condition(arena, condition, gated, &zero)?
        }
        NodePayload::Binop(Binop::Smulp | Binop::Umulp, _, _) => {
            let [Value::Bits(lhs), Value::Bits(rhs)] = operands else {
                return Err(symex_error("partial multiply operands must be bits-typed"));
            };
            let Type::Tuple(fields) = &node.ty else {
                return Err(symex_error("partial multiply result must be tuple-typed"));
            };
            let Some(Type::Bits(result_width)) = fields.first().map(Box::as_ref) else {
                return Err(symex_error(
                    "partial multiply tuple fields must be bits-typed",
                ));
            };
            if *result_width == 0 {
                return Ok(Value::Tuple(vec![
                    Value::Bits(bits(0, None)?),
                    Value::Bits(bits(0, None)?),
                ]));
            }
            let signed = matches!(node.payload, NodePayload::Binop(Binop::Smulp, _, _));
            let lhs = resize(arena, lhs, *result_width, signed)?;
            let rhs = resize(arena, rhs, *result_width, signed)?;
            let product = arena.bit_binary(BitBinaryOp::Mul, bits_expr(&lhs)?, bits_expr(&rhs)?);
            let offset_bits = mulp_offset(*result_width);
            let offset = arena.bits_const(&offset_bits);
            let residual = arena.bit_binary(BitBinaryOp::Sub, product, offset);
            Value::Tuple(vec![
                Value::Bits(bits(*result_width, Some(offset))?),
                Value::Bits(bits(*result_width, Some(residual))?),
            ])
        }
        NodePayload::Binop(op, _, _) => {
            let [Value::Bits(lhs), Value::Bits(rhs)] = operands else {
                return Err(symex_error("binary operands must be bits-typed"));
            };
            Value::Bits(evaluate_binop(arena, *op, lhs, rhs, bits_width(&node.ty)?)?)
        }
        NodePayload::Unop(Unop::Identity, _) => operands
            .first()
            .cloned()
            .ok_or_else(|| symex_error("identity operand is absent"))?,
        NodePayload::Unop(op, _) => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("unary operand must be bits-typed"));
            };
            Value::Bits(evaluate_unop(arena, *op, arg, bits_width(&node.ty)?)?)
        }
        NodePayload::Nary(op, _) => {
            let bits_operands = operands
                .iter()
                .map(|operand| match operand {
                    Value::Bits(bits) => Ok(bits),
                    Value::Tuple(_) | Value::Array(_) => {
                        Err(symex_error("n-ary operand must be bits-typed"))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            Value::Bits(evaluate_nary(
                arena,
                *op,
                &bits_operands,
                bits_width(&node.ty)?,
            )?)
        }
        NodePayload::ZeroExt { new_bit_count, .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("zero_ext operand must be bits-typed"));
            };
            Value::Bits(resize(arena, arg, *new_bit_count, false)?)
        }
        NodePayload::SignExt { new_bit_count, .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("sign_ext operand must be bits-typed"));
            };
            Value::Bits(resize(arena, arg, *new_bit_count, true)?)
        }
        NodePayload::BitSlice { start, width, .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("bit_slice operand must be bits-typed"));
            };
            Value::Bits(if *width == 0 {
                bits(0, None)?
            } else {
                bits(*width, Some(arena.extract(bits_expr(arg)?, *start, *width)))?
            })
        }
        NodePayload::DynamicBitSlice { width, .. } => {
            let [Value::Bits(arg), Value::Bits(start)] = operands else {
                return Err(symex_error("dynamic_bit_slice operands must be bits-typed"));
            };
            Value::Bits(dynamic_bit_slice(arena, arg, start, *width)?)
        }
        NodePayload::BitSliceUpdate { .. } => {
            let [Value::Bits(arg), Value::Bits(start), Value::Bits(update)] = operands else {
                return Err(symex_error("bit_slice_update operands must be bits-typed"));
            };
            Value::Bits(bit_slice_update(arena, arg, start, update)?)
        }
        NodePayload::ArrayIndex { indices, .. } => {
            let Some((array, index_values)) = operands.split_first() else {
                return Err(symex_error("array_index has no array operand"));
            };
            if index_values.len() != indices.len() {
                return Err(symex_error("array_index operand count mismatch"));
            }
            let mut value = array.clone();
            for index in index_values {
                let Value::Bits(index) = index else {
                    return Err(symex_error("array_index index must be bits-typed"));
                };
                let Value::Array(elements) = value else {
                    return Err(symex_error("array_index depth exceeds array type"));
                };
                let last = elements
                    .last()
                    .ok_or_else(|| symex_error("array_index does not support an empty array"))?;
                value = select_structural(arena, index, &elements, Some(last))?;
            }
            value
        }
        NodePayload::ArrayUpdate { indices, .. } => {
            let Some((array, tail)) = operands.split_first() else {
                return Err(symex_error("array_update has no array operand"));
            };
            let Some((update, index_values)) = tail.split_first() else {
                return Err(symex_error("array_update has no update value"));
            };
            if index_values.len() != indices.len() {
                return Err(symex_error("array_update operand count mismatch"));
            }
            let index_values = index_values
                .iter()
                .map(|value| match value {
                    Value::Bits(bits) => Ok(bits),
                    Value::Tuple(_) | Value::Array(_) => {
                        Err(symex_error("array_update index must be bits-typed"))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            array_update(arena, array, update, &index_values)?
        }
        NodePayload::OneHot { lsb_prio, .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("one_hot operand must be bits-typed"));
            };
            Value::Bits(evaluate_one_hot(arena, arg, *lsb_prio)?)
        }
        NodePayload::Encode { .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("encode operand must be bits-typed"));
            };
            Value::Bits(evaluate_encode(arena, arg, bits_width(&node.ty)?)?)
        }
        NodePayload::Decode { width, .. } => {
            let [Value::Bits(arg)] = operands else {
                return Err(symex_error("decode operand must be bits-typed"));
            };
            Value::Bits(evaluate_decode(arena, arg, *width)?)
        }
        NodePayload::Sel { cases, default, .. } => {
            let selector = operand_bits(operands, 0, "sel selector")?;
            let case_values = &operands[1..1 + cases.len()];
            let default_value = default.map(|_| &operands[1 + cases.len()]);
            select_structural(arena, selector, case_values, default_value)?
        }
        NodePayload::PrioritySel { cases, default, .. } => {
            let selector = operand_bits(operands, 0, "priority_sel selector")?;
            let case_values = &operands[1..1 + cases.len()];
            let default_value = default
                .map(|_| &operands[1 + cases.len()])
                .ok_or_else(|| symex_error("priority_sel requires a default"))?;
            priority_select(arena, selector, case_values, default_value)?
        }
        NodePayload::OneHotSel { cases, .. } => {
            let selector = operand_bits(operands, 0, "one_hot_sel selector")?;
            one_hot_select(arena, selector, &operands[1..1 + cases.len()], &node.ty)?
        }
        NodePayload::ExtCarryOut { .. }
        | NodePayload::ExtPrioEncode { .. }
        | NodePayload::ExtClz { .. }
        | NodePayload::ExtNormalizeLeft { .. }
        | NodePayload::ExtMaskLow { .. }
        | NodePayload::ExtNaryAdd { .. } => {
            return Err(symex_error(
                "xlsynth-pir extension operation was not desugared",
            ));
        }
        NodePayload::Nil
        | NodePayload::GetParam(_)
        | NodePayload::Invoke { .. }
        | NodePayload::CountedFor { .. }
        | NodePayload::AfterAll(_)
        | NodePayload::Assert { .. }
        | NodePayload::Trace { .. }
        | NodePayload::Cover { .. }
        | NodePayload::InstantiationInput { .. }
        | NodePayload::InstantiationOutput { .. }
        | NodePayload::RegisterRead { .. }
        | NodePayload::RegisterWrite { .. } => {
            return Err(symex_error(format!(
                "operation {} is not a pure value node",
                node.payload.get_operator()
            )));
        }
    };
    Ok(value)
}

fn operand_bits<'a>(
    operands: &'a [Value],
    index: usize,
    description: &str,
) -> Result<&'a BitsValue, XlsynthError> {
    match operands.get(index) {
        Some(Value::Bits(bits)) => Ok(bits),
        Some(Value::Tuple(_) | Value::Array(_)) => {
            Err(symex_error(format!("{description} must be bits-typed")))
        }
        None => Err(symex_error(format!("{description} is absent"))),
    }
}

fn dynamic_bit_slice(
    arena: &mut ExprArena,
    arg: &BitsValue,
    start: &BitsValue,
    width: usize,
) -> Result<BitsValue, XlsynthError> {
    if width == 0 {
        return bits(0, None);
    }
    let zero_fill_width = arg
        .bit_count
        .checked_add(width)
        .ok_or_else(|| symex_error("dynamic bit-slice width overflow"))?;
    let working_width = zero_fill_width.max(start.bit_count);
    let extended_arg = resize(arena, arg, working_width, false)?;
    let extended_start = resize(arena, start, working_width, false)?;
    let shifted = arena.bit_binary(
        BitBinaryOp::Lshr,
        bits_expr(&extended_arg)?,
        bits_expr(&extended_start)?,
    );
    bits(width, Some(arena.extract(shifted, 0, width)))
}

fn bit_slice_update(
    arena: &mut ExprArena,
    arg: &BitsValue,
    start: &BitsValue,
    update: &BitsValue,
) -> Result<BitsValue, XlsynthError> {
    if arg.bit_count == 0 {
        return bits(0, None);
    }
    let mut result = bits_expr(arg)?;
    for start_index in (0..arg.bit_count).rev() {
        let write_width = update.bit_count.min(arg.bit_count - start_index);
        let mut parts = Vec::new();
        let high_start = start_index + write_width;
        if high_start < arg.bit_count {
            parts.push(arena.extract(bits_expr(arg)?, high_start, arg.bit_count - high_start));
        }
        if write_width > 0 {
            parts.push(if write_width == update.bit_count {
                bits_expr(update)?
            } else {
                arena.extract(bits_expr(update)?, 0, write_width)
            });
        }
        if start_index > 0 {
            parts.push(arena.extract(bits_expr(arg)?, 0, start_index));
        }
        let updated = match parts.as_slice() {
            [] => bits_expr(arg)?,
            [only] => *only,
            _ => arena.concat(parts),
        };
        let condition = selector_equals(arena, start, start_index)?;
        result = arena.ite(condition, updated, result);
    }
    bits(arg.bit_count, Some(result))
}

fn array_update(
    arena: &mut ExprArena,
    array: &Value,
    update: &Value,
    indices: &[&BitsValue],
) -> Result<Value, XlsynthError> {
    if indices.is_empty() {
        return Ok(update.clone());
    }
    let Value::Array(elements) = array else {
        return Err(symex_error("array_update depth exceeds array type"));
    };
    Ok(Value::Array(
        elements
            .iter()
            .enumerate()
            .map(|(index, element)| {
                let candidate = array_update(arena, element, update, &indices[1..])?;
                let condition = selector_equals(arena, indices[0], index)?;
                select_condition(arena, condition, &candidate, element)
            })
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn priority_select(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[Value],
    default: &Value,
) -> Result<Value, XlsynthError> {
    let mut result = default.clone();
    for (index, case) in cases.iter().enumerate().rev() {
        if index >= selector.bit_count {
            continue;
        }
        let bit = arena.extract(bits_expr(selector)?, index, 1);
        let selected = arena.bit_is_one(bit);
        result = select_condition(arena, selected, case, &result)?;
    }
    Ok(result)
}

fn one_hot_select(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[Value],
    result_type: &Type,
) -> Result<Value, XlsynthError> {
    let mut result = zero_value(arena, result_type)?;
    for (index, case) in cases.iter().enumerate() {
        if index >= selector.bit_count {
            continue;
        }
        let bit = arena.extract(bits_expr(selector)?, index, 1);
        let selected = arena.bit_is_one(bit);
        let zero = zero_value(arena, result_type)?;
        let selected_value = select_condition(arena, selected, case, &zero)?;
        result = deep_or(arena, &result, &selected_value)?;
    }
    Ok(result)
}

pub(crate) fn zero_value(arena: &mut ExprArena, ty: &Type) -> Result<Value, XlsynthError> {
    match ty {
        Type::Bits(0) => Ok(Value::Bits(bits(0, None)?)),
        Type::Bits(width) => Ok(Value::Bits(bits(
            *width,
            Some(arena.bits_const_u64(*width, 0)),
        )?)),
        Type::Tuple(types) => Ok(Value::Tuple(
            types
                .iter()
                .map(|ty| zero_value(arena, ty))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Array(array) => Ok(Value::Array(
            (0..array.element_count)
                .map(|_| zero_value(arena, &array.element_type))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Token => Err(symex_error("token values are outside the pure value scope")),
    }
}

pub(crate) fn deep_or(
    arena: &mut ExprArena,
    lhs: &Value,
    rhs: &Value,
) -> Result<Value, XlsynthError> {
    match (lhs, rhs) {
        (Value::Bits(lhs), Value::Bits(rhs)) => {
            equal_widths(lhs, rhs)?;
            if lhs.bit_count == 0 {
                Ok(Value::Bits(bits(0, None)?))
            } else {
                Ok(Value::Bits(bits(
                    lhs.bit_count,
                    Some(arena.bit_binary(BitBinaryOp::Or, bits_expr(lhs)?, bits_expr(rhs)?)),
                )?))
            }
        }
        (Value::Tuple(lhs), Value::Tuple(rhs)) => {
            if lhs.len() != rhs.len() {
                return Err(symex_error("deep-or structural arity mismatch"));
            }
            Ok(Value::Tuple(
                lhs.iter()
                    .zip(rhs)
                    .map(|(lhs, rhs)| deep_or(arena, lhs, rhs))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        (Value::Array(lhs), Value::Array(rhs)) => {
            if lhs.len() != rhs.len() {
                return Err(symex_error("deep-or structural arity mismatch"));
            }
            Ok(Value::Array(
                lhs.iter()
                    .zip(rhs)
                    .map(|(lhs, rhs)| deep_or(arena, lhs, rhs))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        _ => Err(symex_error("deep-or values have different structures")),
    }
}

fn mulp_offset(result_width: usize) -> xlsynth::IrBits {
    let low_width = result_width.saturating_sub(2);
    let high_width = result_width - low_width;
    let low_shift = low_width.saturating_sub(1).min(3);
    let mut bits = vec![false; result_width];
    for bit in bits.iter_mut().take(low_width.saturating_sub(low_shift)) {
        *bit = true;
    }
    for bit in bits
        .iter_mut()
        .skip(low_width)
        .take(high_width.saturating_sub(1))
    {
        *bit = true;
    }
    xlsynth::IrBits::from_lsb_is_0(&bits)
}

fn evaluate_decode(
    arena: &mut ExprArena,
    arg: &BitsValue,
    width: usize,
) -> Result<BitsValue, XlsynthError> {
    if width == 0 {
        return bits(0, None);
    }
    let mut output = (0..width)
        .map(|index| {
            let condition = selector_equals(arena, arg, index)?;
            Ok(arena.bool_to_bit(condition))
        })
        .collect::<Result<Vec<_>, XlsynthError>>()?;
    output.reverse();
    bits(width, Some(arena.concat(output)))
}

fn evaluate_one_hot(
    arena: &mut ExprArena,
    arg: &BitsValue,
    lsb_prio: bool,
) -> Result<BitsValue, XlsynthError> {
    if arg.bit_count == 0 {
        return Err(symex_error("one_hot requires a nonempty operand"));
    }
    let arg_expr = bits_expr(arg)?;
    let zero = arena.bits_const_u64(arg.bit_count, 0);
    let is_zero = arena.compare(CompareOp::Eq, arg_expr, zero);
    let mut output_bits = Vec::with_capacity(arg.bit_count + 1);
    for index in 0..arg.bit_count {
        let mut selected = arena.extract(arg_expr, index, 1);
        let higher_priority = if lsb_prio {
            (0..index).collect::<Vec<_>>()
        } else {
            (index + 1..arg.bit_count).collect::<Vec<_>>()
        };
        for prior in higher_priority {
            let prior_bit = arena.extract(arg_expr, prior, 1);
            let not_prior = arena.bit_unary(BitUnaryOp::Not, prior_bit);
            selected = arena.bit_binary(BitBinaryOp::And, selected, not_prior);
        }
        output_bits.push(selected);
    }
    output_bits.push(arena.bool_to_bit(is_zero));
    output_bits.reverse();
    bits(arg.bit_count + 1, Some(arena.concat(output_bits)))
}

fn evaluate_encode(
    arena: &mut ExprArena,
    arg: &BitsValue,
    result_width: usize,
) -> Result<BitsValue, XlsynthError> {
    if result_width == 0 {
        return bits(0, None);
    }
    let mut expression = arena.bits_const_u64(result_width, 0);
    for index in 0..arg.bit_count {
        let bit = arena.extract(bits_expr(arg)?, index, 1);
        let selected = arena.bit_is_one(bit);
        let index_expr = arena.bits_const_u64(result_width, index as u64);
        let with_index = arena.bit_binary(BitBinaryOp::Or, expression, index_expr);
        expression = arena.ite(selected, with_index, expression);
    }
    bits(result_width, Some(expression))
}

fn evaluate_binop(
    arena: &mut ExprArena,
    op: Binop,
    lhs: &BitsValue,
    rhs: &BitsValue,
    result_width: usize,
) -> Result<BitsValue, XlsynthError> {
    if result_width == 0 {
        return bits(0, None);
    }
    if lhs.bit_count == 0 || rhs.bit_count == 0 {
        equal_widths(lhs, rhs)?;
        let value = matches!(
            op,
            Binop::Eq | Binop::Uge | Binop::Ule | Binop::Sge | Binop::Sle
        );
        return bits(1, Some(arena.bits_const_u64(1, u64::from(value))));
    }
    let expression = match op {
        Binop::Add => binary(arena, BitBinaryOp::Add, lhs, rhs)?,
        Binop::Sub => binary(arena, BitBinaryOp::Sub, lhs, rhs)?,
        Binop::Umul => {
            let lhs = resize(arena, lhs, result_width, false)?;
            let rhs = resize(arena, rhs, result_width, false)?;
            binary(arena, BitBinaryOp::Mul, &lhs, &rhs)?
        }
        Binop::Smul => {
            let lhs = resize(arena, lhs, result_width, true)?;
            let rhs = resize(arena, rhs, result_width, true)?;
            binary(arena, BitBinaryOp::Mul, &lhs, &rhs)?
        }
        Binop::Udiv => xls_div(arena, lhs, rhs, false)?,
        Binop::Sdiv => xls_div(arena, lhs, rhs, true)?,
        Binop::Umod => xls_mod(arena, lhs, rhs, false)?,
        Binop::Smod => xls_mod(arena, lhs, rhs, true)?,
        Binop::Eq => comparison(arena, CompareOp::Eq, lhs, rhs, false)?,
        Binop::Ne => comparison(arena, CompareOp::Eq, lhs, rhs, true)?,
        Binop::Uge => comparison(arena, CompareOp::Uge, lhs, rhs, false)?,
        Binop::Ugt => comparison(arena, CompareOp::Ugt, lhs, rhs, false)?,
        Binop::Ult => comparison(arena, CompareOp::Ult, lhs, rhs, false)?,
        Binop::Ule => comparison(arena, CompareOp::Ule, lhs, rhs, false)?,
        Binop::Sge => comparison(arena, CompareOp::Sge, lhs, rhs, false)?,
        Binop::Sgt => comparison(arena, CompareOp::Sgt, lhs, rhs, false)?,
        Binop::Slt => comparison(arena, CompareOp::Slt, lhs, rhs, false)?,
        Binop::Sle => comparison(arena, CompareOp::Sle, lhs, rhs, false)?,
        Binop::Shll => shift(arena, BitBinaryOp::Shl, lhs, rhs)?,
        Binop::Shrl => shift(arena, BitBinaryOp::Lshr, lhs, rhs)?,
        Binop::Shra => shift(arena, BitBinaryOp::Ashr, lhs, rhs)?,
        _ => return Err(symex_error(format!("unsupported binary operation {op:?}"))),
    };
    bits(result_width, Some(expression))
}

fn evaluate_unop(
    arena: &mut ExprArena,
    op: Unop,
    arg: &BitsValue,
    result_width: usize,
) -> Result<BitsValue, XlsynthError> {
    if arg.bit_count == 0 {
        let expression = match op {
            Unop::OrReduce | Unop::XorReduce => Some(arena.bits_const_u64(1, 0)),
            Unop::AndReduce => Some(arena.bits_const_u64(1, 1)),
            Unop::Identity | Unop::Neg | Unop::Not | Unop::Reverse => None,
        };
        return bits(result_width, expression);
    }
    let arg_expr = bits_expr(arg)?;
    let expression = match op {
        Unop::Identity => arg_expr,
        Unop::Neg => arena.bit_unary(BitUnaryOp::Neg, arg_expr),
        Unop::Not => arena.bit_unary(BitUnaryOp::Not, arg_expr),
        Unop::OrReduce => {
            let zero = arena.bits_const_u64(arg.bit_count, 0);
            let is_zero = arena.compare(CompareOp::Eq, arg_expr, zero);
            let nonzero = arena.bool_not(is_zero);
            arena.bool_to_bit(nonzero)
        }
        Unop::AndReduce => {
            let ones = xlsynth::IrBits::all_ones(arg.bit_count);
            let ones = arena.bits_const(&ones);
            let all_ones = arena.compare(CompareOp::Eq, arg_expr, ones);
            arena.bool_to_bit(all_ones)
        }
        Unop::XorReduce => {
            let mut expression = arena.extract(arg_expr, 0, 1);
            for index in 1..arg.bit_count {
                let bit = arena.extract(arg_expr, index, 1);
                expression = arena.bit_binary(BitBinaryOp::Xor, expression, bit);
            }
            expression
        }
        Unop::Reverse => {
            let mut output = (0..arg.bit_count)
                .map(|index| arena.extract(arg_expr, index, 1))
                .collect::<Vec<_>>();
            if output.len() == 1 {
                output[0]
            } else {
                arena.concat(output.split_off(0))
            }
        }
    };
    bits(result_width, Some(expression))
}

fn evaluate_nary(
    arena: &mut ExprArena,
    op: NaryOp,
    operands: &[&BitsValue],
    result_width: usize,
) -> Result<BitsValue, XlsynthError> {
    if result_width == 0 {
        return bits(0, None);
    }
    let operands = if op == NaryOp::Concat {
        operands
            .iter()
            .copied()
            .filter(|operand| operand.bit_count > 0)
            .collect::<Vec<_>>()
    } else {
        operands.to_vec()
    };
    if operands.is_empty() {
        return Err(symex_error("empty n-ary operation is unsupported"));
    }
    if op != NaryOp::Concat
        && operands
            .iter()
            .any(|operand| operand.bit_count != result_width)
    {
        return Err(symex_error("n-ary operand widths do not match result"));
    }
    let expression = if op == NaryOp::Concat {
        arena.concat(
            operands
                .iter()
                .map(|operand| bits_expr(operand))
                .collect::<Result<Vec<_>, _>>()?,
        )
    } else {
        let (binary_op, negate) = match op {
            NaryOp::And => (BitBinaryOp::And, false),
            NaryOp::Nand => (BitBinaryOp::And, true),
            NaryOp::Or => (BitBinaryOp::Or, false),
            NaryOp::Nor => (BitBinaryOp::Or, true),
            NaryOp::Xor => (BitBinaryOp::Xor, false),
            _ => return Err(symex_error(format!("unsupported n-ary operation {op:?}"))),
        };
        let mut expressions = operands.iter().map(|operand| bits_expr(operand));
        let mut result = expressions.next().transpose()?.unwrap();
        for operand in expressions {
            result = arena.bit_binary(binary_op, result, operand?);
        }
        if negate {
            arena.bit_unary(BitUnaryOp::Not, result)
        } else {
            result
        }
    };
    bits(result_width, Some(expression))
}

fn select_structural(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[Value],
    default: Option<&Value>,
) -> Result<Value, XlsynthError> {
    let first = cases
        .first()
        .ok_or_else(|| symex_error("select has no cases"))?;
    match first {
        Value::Bits(first_bits) => {
            let case_bits = cases
                .iter()
                .map(|case| match case {
                    Value::Bits(bits) => Ok(bits),
                    _ => Err(symex_error("select cases have different structures")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let default_bits = match default {
                Some(Value::Bits(bits)) => Some(bits),
                Some(_) => return Err(symex_error("select default has different structure")),
                None => None,
            };
            Ok(Value::Bits(evaluate_sel(
                arena,
                selector,
                &case_bits,
                default_bits,
                first_bits.bit_count,
            )?))
        }
        Value::Tuple(first_elements) => {
            let mut result = Vec::with_capacity(first_elements.len());
            for index in 0..first_elements.len() {
                let element_cases = cases
                    .iter()
                    .map(|case| match case {
                        Value::Tuple(elements) => elements
                            .get(index)
                            .cloned()
                            .ok_or_else(|| symex_error("select tuple arity mismatch")),
                        _ => Err(symex_error("select cases have different structures")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_element = match default {
                    Some(Value::Tuple(elements)) => elements.get(index),
                    Some(_) => return Err(symex_error("select default has different structure")),
                    None => None,
                };
                result.push(select_structural(
                    arena,
                    selector,
                    &element_cases,
                    default_element,
                )?);
            }
            Ok(Value::Tuple(result))
        }
        Value::Array(first_elements) => {
            let mut result = Vec::with_capacity(first_elements.len());
            for index in 0..first_elements.len() {
                let element_cases = cases
                    .iter()
                    .map(|case| match case {
                        Value::Array(elements) => elements
                            .get(index)
                            .cloned()
                            .ok_or_else(|| symex_error("select array length mismatch")),
                        _ => Err(symex_error("select cases have different structures")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_element = match default {
                    Some(Value::Array(elements)) => elements.get(index),
                    Some(_) => return Err(symex_error("select default has different structure")),
                    None => None,
                };
                result.push(select_structural(
                    arena,
                    selector,
                    &element_cases,
                    default_element,
                )?);
            }
            Ok(Value::Array(result))
        }
    }
}

fn select_condition(
    arena: &mut ExprArena,
    condition: ExprId,
    then_value: &Value,
    else_value: &Value,
) -> Result<Value, XlsynthError> {
    match (then_value, else_value) {
        (Value::Bits(then_bits), Value::Bits(else_bits)) => {
            equal_widths(then_bits, else_bits)?;
            if then_bits.bit_count == 0 {
                return Ok(Value::Bits(bits(0, None)?));
            }
            Ok(Value::Bits(bits(
                then_bits.bit_count,
                Some(arena.ite(condition, bits_expr(then_bits)?, bits_expr(else_bits)?)),
            )?))
        }
        (Value::Tuple(then_elements), Value::Tuple(else_elements))
        | (Value::Array(then_elements), Value::Array(else_elements)) => {
            if then_elements.len() != else_elements.len() {
                return Err(symex_error("conditional structural arity mismatch"));
            }
            let elements = then_elements
                .iter()
                .zip(else_elements)
                .map(|(then_element, else_element)| {
                    select_condition(arena, condition, then_element, else_element)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if matches!(then_value, Value::Tuple(_)) {
                Ok(Value::Tuple(elements))
            } else {
                Ok(Value::Array(elements))
            }
        }
        _ => Err(symex_error("conditional values have different structures")),
    }
}

fn evaluate_sel(
    arena: &mut ExprArena,
    selector: &BitsValue,
    cases: &[&BitsValue],
    default: Option<&BitsValue>,
    result_width: usize,
) -> Result<BitsValue, XlsynthError> {
    if result_width == 0 {
        return bits(0, None);
    }
    if cases.is_empty() {
        return Err(symex_error("select has no cases"));
    }
    if cases.iter().any(|case| case.bit_count != result_width)
        || default.is_some_and(|value| value.bit_count != result_width)
    {
        return Err(symex_error("select case widths do not match result"));
    }
    let mut expression = bits_expr(default.unwrap_or(cases.last().unwrap()))?;
    for (index, case) in cases.iter().enumerate().rev() {
        let condition = selector_equals(arena, selector, index)?;
        expression = arena.ite(condition, bits_expr(case)?, expression);
    }
    bits(result_width, Some(expression))
}

pub(crate) fn selector_equals(
    arena: &mut ExprArena,
    selector: &BitsValue,
    index: usize,
) -> Result<ExprId, XlsynthError> {
    if selector.bit_count == 0 {
        return Ok(arena.bool_const(index == 0));
    }
    if selector.bit_count < usize::BITS as usize && index >= (1_usize << selector.bit_count) {
        return Ok(arena.bool_const(false));
    }
    let value = (0..selector.bit_count)
        .map(|bit| bit < usize::BITS as usize && index & (1_usize << bit) != 0)
        .collect::<Vec<_>>();
    let index_expr = arena.bits_const(&xlsynth::IrBits::from_lsb_is_0(&value));
    Ok(arena.compare(CompareOp::Eq, bits_expr(selector)?, index_expr))
}

fn binary(
    arena: &mut ExprArena,
    operation: BitBinaryOp,
    lhs: &BitsValue,
    rhs: &BitsValue,
) -> Result<ExprId, XlsynthError> {
    equal_widths(lhs, rhs)?;
    Ok(arena.bit_binary(operation, bits_expr(lhs)?, bits_expr(rhs)?))
}

fn comparison(
    arena: &mut ExprArena,
    operation: CompareOp,
    lhs: &BitsValue,
    rhs: &BitsValue,
    negate: bool,
) -> Result<ExprId, XlsynthError> {
    equal_widths(lhs, rhs)?;
    let condition = arena.compare(operation, bits_expr(lhs)?, bits_expr(rhs)?);
    let condition = if negate {
        arena.bool_not(condition)
    } else {
        condition
    };
    Ok(arena.bool_to_bit(condition))
}

fn xls_div(
    arena: &mut ExprArena,
    lhs: &BitsValue,
    rhs: &BitsValue,
    signed: bool,
) -> Result<ExprId, XlsynthError> {
    equal_widths(lhs, rhs)?;
    let width = lhs.bit_count;
    let lhs_expr = bits_expr(lhs)?;
    let rhs_expr = bits_expr(rhs)?;
    let zero = arena.bits_const_u64(width, 0);
    let rhs_is_zero = arena.compare(CompareOp::Eq, rhs_expr, zero);
    let quotient = arena.bit_binary(
        if signed {
            BitBinaryOp::Sdiv
        } else {
            BitBinaryOp::Udiv
        },
        lhs_expr,
        rhs_expr,
    );
    let fallback = if signed {
        let sign = arena.extract(lhs_expr, width - 1, 1);
        let negative = arena.bit_is_one(sign);
        let min = arena.bits_const(&xlsynth::IrBits::signed_min_value(width));
        let max = arena.bits_const(&xlsynth::IrBits::signed_max_value(width));
        arena.ite(negative, min, max)
    } else {
        arena.bits_const(&xlsynth::IrBits::all_ones(width))
    };
    Ok(arena.ite(rhs_is_zero, fallback, quotient))
}

fn xls_mod(
    arena: &mut ExprArena,
    lhs: &BitsValue,
    rhs: &BitsValue,
    signed: bool,
) -> Result<ExprId, XlsynthError> {
    equal_widths(lhs, rhs)?;
    let lhs_expr = bits_expr(lhs)?;
    let rhs_expr = bits_expr(rhs)?;
    let zero = arena.bits_const_u64(lhs.bit_count, 0);
    let rhs_is_zero = arena.compare(CompareOp::Eq, rhs_expr, zero);
    let remainder = arena.bit_binary(
        if signed {
            BitBinaryOp::Srem
        } else {
            BitBinaryOp::Urem
        },
        lhs_expr,
        rhs_expr,
    );
    Ok(arena.ite(rhs_is_zero, zero, remainder))
}

fn shift(
    arena: &mut ExprArena,
    operation: BitBinaryOp,
    lhs: &BitsValue,
    rhs: &BitsValue,
) -> Result<ExprId, XlsynthError> {
    let result_width = lhs.bit_count;
    let working_width = lhs.bit_count.max(rhs.bit_count);
    let lhs = resize(arena, lhs, working_width, operation == BitBinaryOp::Ashr)?;
    let rhs = resize(arena, rhs, working_width, false)?;
    let shifted = arena.bit_binary(operation, bits_expr(&lhs)?, bits_expr(&rhs)?);
    if working_width == result_width {
        Ok(shifted)
    } else {
        Ok(arena.extract(shifted, 0, result_width))
    }
}

fn resize(
    arena: &mut ExprArena,
    value: &BitsValue,
    width: usize,
    signed: bool,
) -> Result<BitsValue, XlsynthError> {
    if value.bit_count == width {
        return Ok(value.clone());
    }
    if width == 0 {
        return bits(0, None);
    }
    if value.bit_count == 0 {
        return bits(width, Some(arena.bits_const_u64(width, 0)));
    }
    if value.bit_count < width {
        return bits(width, Some(arena.extend(bits_expr(value)?, width, signed)));
    }
    bits(width, Some(arena.extract(bits_expr(value)?, 0, width)))
}

fn equal_widths(lhs: &BitsValue, rhs: &BitsValue) -> Result<(), XlsynthError> {
    if lhs.bit_count == rhs.bit_count {
        Ok(())
    } else {
        Err(symex_error(format!(
            "operand width mismatch: {} and {}",
            lhs.bit_count, rhs.bit_count
        )))
    }
}

fn get_value(values: &[Option<Value>], node_ref: NodeRef) -> Result<&Value, XlsynthError> {
    values
        .get(node_ref.index)
        .and_then(Option::as_ref)
        .ok_or_else(|| {
            symex_error(format!(
                "operand node {} has no symbolic value",
                node_ref.index
            ))
        })
}

fn get_bits(values: &[Option<Value>], node_ref: NodeRef) -> Result<&BitsValue, XlsynthError> {
    match get_value(values, node_ref)? {
        Value::Bits(bits) => Ok(bits),
        Value::Tuple(_) | Value::Array(_) => Err(symex_error(format!(
            "operand node {} is structured, expected bits",
            node_ref.index
        ))),
    }
}

pub(crate) fn symbolic_input(
    arena: &mut ExprArena,
    ty: &Type,
    name: &str,
    parameters: &mut Vec<SymbolicParameter>,
) -> Result<Value, XlsynthError> {
    match ty {
        Type::Bits(0) => Ok(Value::Bits(bits(0, None)?)),
        Type::Bits(bit_count) => {
            parameters.push(SymbolicParameter {
                name: name.to_owned(),
                bit_count: *bit_count,
            });
            Ok(Value::Bits(bits(
                *bit_count,
                Some(arena.variable(name, Sort::Bits(*bit_count))),
            )?))
        }
        Type::Tuple(element_types) => Ok(Value::Tuple(
            element_types
                .iter()
                .enumerate()
                .map(|(index, element_type)| {
                    symbolic_input(arena, element_type, &format!("{name}_{index}"), parameters)
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Array(array) => Ok(Value::Array(
            (0..array.element_count)
                .map(|index| {
                    symbolic_input(
                        arena,
                        &array.element_type,
                        &format!("{name}_{index}"),
                        parameters,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Token => Err(symex_error(
            "token parameters are outside the pure value scope",
        )),
    }
}

pub(crate) fn evaluation_input(
    arena: &mut ExprArena,
    ty: &Type,
    input: &EvaluationInput,
    name: &str,
    parameters: &mut Vec<SymbolicParameter>,
) -> Result<Value, XlsynthError> {
    match input {
        EvaluationInput::Symbolic => symbolic_input(arena, ty, name, parameters),
        EvaluationInput::Concrete(value) => literal_value(arena, ty, value),
        EvaluationInput::Tuple(inputs) => {
            let Type::Tuple(element_types) = ty else {
                return Err(symex_error(format!(
                    "tuple input supplied for non-tuple type {ty}"
                )));
            };
            if inputs.len() != element_types.len() {
                return Err(symex_error(format!(
                    "tuple input has {} elements, expected {}",
                    inputs.len(),
                    element_types.len()
                )));
            }
            Ok(Value::Tuple(
                element_types
                    .iter()
                    .zip(inputs)
                    .enumerate()
                    .map(|(index, (element_type, input))| {
                        evaluation_input(
                            arena,
                            element_type,
                            input,
                            &format!("{name}_{index}"),
                            parameters,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        EvaluationInput::Array(inputs) => {
            let Type::Array(array_type) = ty else {
                return Err(symex_error(format!(
                    "array input supplied for non-array type {ty}"
                )));
            };
            if inputs.len() != array_type.element_count {
                return Err(symex_error(format!(
                    "array input has {} elements, expected {}",
                    inputs.len(),
                    array_type.element_count
                )));
            }
            Ok(Value::Array(
                inputs
                    .iter()
                    .enumerate()
                    .map(|(index, input)| {
                        evaluation_input(
                            arena,
                            &array_type.element_type,
                            input,
                            &format!("{name}_{index}"),
                            parameters,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
    }
}

fn literal_value(arena: &mut ExprArena, ty: &Type, value: &IrValue) -> Result<Value, XlsynthError> {
    match ty {
        Type::Bits(0) => Ok(Value::Bits(bits(0, None)?)),
        Type::Bits(bit_count) => {
            let ir_bits = value.to_bits()?;
            if ir_bits.get_bit_count() != *bit_count {
                return Err(symex_error("bits literal width does not match node type"));
            }
            Ok(Value::Bits(bits(
                *bit_count,
                Some(arena.bits_const(&ir_bits)),
            )?))
        }
        Type::Tuple(element_types) => {
            let elements = value.get_elements()?;
            if elements.len() != element_types.len() {
                return Err(symex_error("tuple literal arity mismatch"));
            }
            Ok(Value::Tuple(
                element_types
                    .iter()
                    .zip(&elements)
                    .map(|(element_type, element)| literal_value(arena, element_type, element))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        Type::Array(array) => {
            let elements = value.get_elements()?;
            if elements.len() != array.element_count {
                return Err(symex_error("array literal length mismatch"));
            }
            Ok(Value::Array(
                elements
                    .iter()
                    .map(|element| literal_value(arena, &array.element_type, element))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        Type::Token => Err(symex_error(
            "token literals are outside the pure value scope",
        )),
    }
}

pub(crate) fn materialize_value(arena: &ExprArena, value: &Value) -> SymbolicValue {
    match value {
        Value::Bits(bits) => SymbolicValue::Bits(SymbolicBits {
            bit_count: bits.bit_count,
            expression: bits
                .expression
                .map(|expression| arena.to_smtlib(expression))
                .unwrap_or_default(),
        }),
        Value::Tuple(elements) => SymbolicValue::Tuple(
            elements
                .iter()
                .map(|element| materialize_value(arena, element))
                .collect(),
        ),
        Value::Array(elements) => SymbolicValue::Array(
            elements
                .iter()
                .map(|element| materialize_value(arena, element))
                .collect(),
        ),
    }
}

pub(crate) fn bits(
    bit_count: usize,
    expression: Option<ExprId>,
) -> Result<BitsValue, XlsynthError> {
    if (bit_count == 0) != expression.is_none() {
        return Err(symex_error(format!(
            "bits[{bit_count}] must {} an expression",
            if bit_count == 0 { "not have" } else { "have" }
        )));
    }
    Ok(BitsValue {
        bit_count,
        expression,
    })
}

pub(crate) fn bits_expr(bits: &BitsValue) -> Result<ExprId, XlsynthError> {
    bits.expression
        .ok_or_else(|| symex_error("bits[0] has no symbolic expression"))
}

pub(crate) fn bits_width(ty: &Type) -> Result<usize, XlsynthError> {
    match ty {
        Type::Bits(bit_count) => Ok(*bit_count),
        _ => Err(symex_error(format!(
            "unsupported symbolic value type: {ty}"
        ))),
    }
}

pub(crate) fn symex_error(message: impl Into<String>) -> XlsynthError {
    XlsynthError(format!("xlsynth-symex: {}", message.into()))
}
