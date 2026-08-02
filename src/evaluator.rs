// SPDX-License-Identifier: Apache-2.0

use xlsynth::{IrValue, XlsynthError};
use xlsynth_pir::ir::{Binop, Fn, NaryOp, NodePayload, NodeRef, Package, Type, Unop};
use xlsynth_pir::ir_parser::Parser;

use crate::{PathCondition, SymbolicBits, SymbolicParameter, SymbolicValue, SymexResult};

pub(crate) fn evaluate_function_text(function_text: &str) -> Result<SymexResult, XlsynthError> {
    let package_text = format!("package standalone\n\n{function_text}");
    let package = Parser::new(&package_text)
        .parse_and_validate_package()
        .map_err(|error| symex_error(format!("failed to parse XLS IR function: {error}")))?;
    let function_name = package
        .get_top_fn()
        .ok_or_else(|| symex_error("standalone IR has no function"))?
        .name
        .clone();
    evaluate_parsed(&package, &function_name)
}

pub(crate) fn evaluate_package_text(
    package_text: &str,
    function_name: &str,
) -> Result<SymexResult, XlsynthError> {
    let package = Parser::new(package_text)
        .parse_and_validate_package()
        .map_err(|error| symex_error(format!("failed to parse XLS IR package: {error}")))?;
    evaluate_parsed(&package, function_name)
}

fn evaluate_parsed(package: &Package, function_name: &str) -> Result<SymexResult, XlsynthError> {
    let function = package
        .get_fn(function_name)
        .ok_or_else(|| symex_error(format!("function {function_name:?} is absent from package")))?;
    let mut parameters = Vec::new();
    let arguments = function
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            symbolic_input(
                &parameter.ty,
                &format!("symex_arg_{index}"),
                &mut parameters,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = Evaluator { package }.evaluate_function(function, arguments)?;
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
        path_condition: PathCondition::True,
        parameters,
        result,
        result_smtlib,
    })
}

struct Evaluator<'a> {
    package: &'a Package,
}

impl Evaluator<'_> {
    fn evaluate_function(
        &self,
        function: &Fn,
        arguments: Vec<SymbolicValue>,
    ) -> Result<SymbolicValue, XlsynthError> {
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
                NodePayload::Literal(value) => literal_value(&node.ty, value).map_err(|error| {
                    symex_error(format!("literal at node {}: {error}", node.text_id))
                })?,
                NodePayload::Tuple(elements) => SymbolicValue::Tuple(
                    elements
                        .iter()
                        .map(|element| get_value(&values, *element).cloned())
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                NodePayload::Array(elements) => SymbolicValue::Array(
                    elements
                        .iter()
                        .map(|element| get_value(&values, *element).cloned())
                        .collect::<Result<Vec<_>, _>>()?,
                ),
                NodePayload::ArrayIndex { array, indices, .. } => {
                    if indices.len() != 1 {
                        return Err(symex_error("only one-dimensional array_index is supported"));
                    }
                    let selector = get_bits(&values, indices[0])?;
                    match get_value(&values, *array)? {
                        SymbolicValue::Array(elements) => {
                            select_structural(selector, elements, None)?
                        }
                        _ => return Err(symex_error("array_index operand is not array-typed")),
                    }
                }
                NodePayload::ArrayUpdate {
                    array,
                    value,
                    indices,
                    ..
                } => {
                    if indices.len() != 1 {
                        return Err(symex_error(
                            "only one-dimensional array_update is supported",
                        ));
                    }
                    let selector = get_bits(&values, indices[0])?;
                    let update_value = get_value(&values, *value)?;
                    match get_value(&values, *array)? {
                        SymbolicValue::Array(elements) => SymbolicValue::Array(
                            elements
                                .iter()
                                .enumerate()
                                .map(|(index, element)| {
                                    let selected = bits(
                                        1,
                                        format!(
                                            "(ite (= {} (_ bv{index} {})) #b1 #b0)",
                                            selector.expression, selector.bit_count
                                        ),
                                    )?;
                                    select_structural(
                                        &selected,
                                        &[element.clone(), update_value.clone()],
                                        None,
                                    )
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        ),
                        _ => return Err(symex_error("array_update operand is not array-typed")),
                    }
                }
                NodePayload::TupleIndex { tuple, index } => match get_value(&values, *tuple)? {
                    SymbolicValue::Tuple(elements) => {
                        elements.get(*index).cloned().ok_or_else(|| {
                            symex_error(format!("tuple index {index} is out of bounds"))
                        })?
                    }
                    SymbolicValue::Bits(_) | SymbolicValue::Array(_) => {
                        return Err(symex_error("tuple_index operand is not tuple-typed"));
                    }
                },
                NodePayload::Binop(op, lhs, rhs) => {
                    let lhs = get_bits(&values, *lhs)?;
                    let rhs = get_bits(&values, *rhs)?;
                    SymbolicValue::Bits(evaluate_binop(*op, lhs, rhs, bits_width(&node.ty)?)?)
                }
                NodePayload::Unop(op, arg) => SymbolicValue::Bits(evaluate_unop(
                    *op,
                    get_bits(&values, *arg)?,
                    bits_width(&node.ty)?,
                )?),
                NodePayload::Nary(op, operands) => {
                    let operands = operands
                        .iter()
                        .map(|operand| get_bits(&values, *operand))
                        .collect::<Result<Vec<_>, _>>()?;
                    SymbolicValue::Bits(evaluate_nary(*op, &operands, bits_width(&node.ty)?)?)
                }
                NodePayload::ZeroExt { arg, new_bit_count } => {
                    let arg = get_bits(&values, *arg)?;
                    SymbolicValue::Bits(extend("zero_extend", arg, *new_bit_count)?)
                }
                NodePayload::SignExt { arg, new_bit_count } => {
                    let arg = get_bits(&values, *arg)?;
                    SymbolicValue::Bits(extend("sign_extend", arg, *new_bit_count)?)
                }
                NodePayload::BitSlice { arg, start, width } => {
                    let arg = get_bits(&values, *arg)?;
                    let expression = if *width == 0 {
                        String::new()
                    } else {
                        format!(
                            "((_ extract {} {}) {})",
                            start + width - 1,
                            start,
                            arg.expression
                        )
                    };
                    SymbolicValue::Bits(bits(*width, expression)?)
                }
                NodePayload::DynamicBitSlice { arg, start, width } => {
                    let arg = get_bits(&values, *arg)?;
                    let start = resize_unsigned(get_bits(&values, *start)?, arg.bit_count)?;
                    if *width == 0 || *width > arg.bit_count {
                        return Err(symex_error(format!(
                            "unsupported dynamic bit-slice width {width} for bits[{}]",
                            arg.bit_count
                        )));
                    }
                    let shifted = format!("(bvlshr {} {})", arg.expression, start.expression);
                    let expression = if *width == arg.bit_count {
                        shifted
                    } else {
                        format!("((_ extract {} 0) {shifted})", width - 1)
                    };
                    SymbolicValue::Bits(bits(*width, expression)?)
                }
                NodePayload::OneHot { arg, lsb_prio } => {
                    SymbolicValue::Bits(evaluate_one_hot(get_bits(&values, *arg)?, *lsb_prio)?)
                }
                NodePayload::Encode { arg } => SymbolicValue::Bits(evaluate_encode(
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
                    select_structural(selector, &cases, default.as_ref())?
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
                        let mut arguments = Vec::with_capacity(2 + invariants.len());
                        arguments.push(SymbolicValue::Bits(bits(
                            induction_width,
                            format!("(_ bv{induction} {induction_width})"),
                        )?));
                        arguments.push(carry);
                        arguments.extend(invariants.iter().cloned());
                        carry = self.evaluate_function(body_function, arguments)?;
                    }
                    carry
                }
                payload => {
                    return Err(symex_error(format!(
                        "unsupported XLS IR operation {} at node {}",
                        payload.get_operator(),
                        node.text_id
                    )));
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

fn evaluate_one_hot(arg: &SymbolicBits, lsb_prio: bool) -> Result<SymbolicBits, XlsynthError> {
    if arg.bit_count == 0 {
        return Err(symex_error("one_hot requires a nonempty operand"));
    }
    let is_zero = format!(
        "(ite (= {} (_ bv0 {})) #b1 #b0)",
        arg.expression, arg.bit_count
    );
    let mut output_bits = Vec::with_capacity(arg.bit_count + 1);
    for index in 0..arg.bit_count {
        let higher_priority = if lsb_prio {
            0..index
        } else {
            index + 1..arg.bit_count
        };
        let mut selected = format!("((_ extract {index} {index}) {})", arg.expression);
        for prior in higher_priority {
            selected = format!(
                "(bvand {selected} (bvnot ((_ extract {prior} {prior}) {})))",
                arg.expression
            );
        }
        output_bits.push(selected);
    }
    output_bits.push(is_zero);
    let expression = output_bits
        .iter()
        .rev()
        .skip(1)
        .fold(output_bits.last().unwrap().clone(), |acc, bit| {
            format!("(concat {acc} {bit})")
        });
    bits(arg.bit_count + 1, expression)
}

fn evaluate_encode(arg: &SymbolicBits, result_width: usize) -> Result<SymbolicBits, XlsynthError> {
    if result_width == 0 {
        return bits(0, String::new());
    }
    let mut expression = format!("(_ bv0 {result_width})");
    for index in 0..arg.bit_count {
        expression = format!(
            "(ite (= ((_ extract {index} {index}) {}) #b1) (bvor {expression} (_ bv{index} {result_width})) {expression})",
            arg.expression
        );
    }
    bits(result_width, expression)
}

fn evaluate_binop(
    op: Binop,
    lhs: &SymbolicBits,
    rhs: &SymbolicBits,
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
    let expression = match op {
        Binop::Add => binary("bvadd", lhs, rhs)?,
        Binop::Sub => binary("bvsub", lhs, rhs)?,
        Binop::Umul => {
            let lhs = resize_unsigned(lhs, result_width)?;
            let rhs = resize_unsigned(rhs, result_width)?;
            binary("bvmul", &lhs, &rhs)?
        }
        Binop::Smul => {
            let lhs = resize_signed(lhs, result_width)?;
            let rhs = resize_signed(rhs, result_width)?;
            binary("bvmul", &lhs, &rhs)?
        }
        Binop::Eq => comparison("=", lhs, rhs)?,
        Binop::Ne => comparison("distinct", lhs, rhs)?,
        Binop::Uge => comparison("bvuge", lhs, rhs)?,
        Binop::Ugt => comparison("bvugt", lhs, rhs)?,
        Binop::Ult => comparison("bvult", lhs, rhs)?,
        Binop::Ule => comparison("bvule", lhs, rhs)?,
        Binop::Sge => comparison("bvsge", lhs, rhs)?,
        Binop::Sgt => comparison("bvsgt", lhs, rhs)?,
        Binop::Slt => comparison("bvslt", lhs, rhs)?,
        Binop::Sle => comparison("bvsle", lhs, rhs)?,
        Binop::Shll => shift("bvshl", lhs, rhs)?,
        Binop::Shrl => shift("bvlshr", lhs, rhs)?,
        Binop::Shra => shift("bvashr", lhs, rhs)?,
        _ => return Err(symex_error(format!("unsupported binary operation {op:?}"))),
    };
    bits(result_width, expression)
}

fn evaluate_unop(
    op: Unop,
    arg: &SymbolicBits,
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
    if arg.bit_count == 0 {
        let expression = match op {
            Unop::OrReduce | Unop::XorReduce => "#b0".to_owned(),
            Unop::AndReduce => "#b1".to_owned(),
            Unop::Identity | Unop::Neg | Unop::Not | Unop::Reverse => String::new(),
        };
        return bits(result_width, expression);
    }
    let expression = match op {
        Unop::Identity => arg.expression.clone(),
        Unop::Neg => format!("(bvneg {})", arg.expression),
        Unop::Not => format!("(bvnot {})", arg.expression),
        Unop::OrReduce => format!(
            "(ite (= {} (_ bv0 {})) #b0 #b1)",
            arg.expression, arg.bit_count
        ),
        Unop::AndReduce => format!(
            "(ite (= {} (bvnot (_ bv0 {}))) #b1 #b0)",
            arg.expression, arg.bit_count
        ),
        Unop::XorReduce => {
            let mut expression = format!("((_ extract 0 0) {})", arg.expression);
            for index in 1..arg.bit_count {
                expression = format!(
                    "(bvxor {expression} ((_ extract {index} {index}) {}))",
                    arg.expression
                );
            }
            expression
        }
        Unop::Reverse => {
            let mut expression = format!("((_ extract 0 0) {})", arg.expression);
            for index in 1..arg.bit_count {
                expression = format!(
                    "(concat {expression} ((_ extract {index} {index}) {}))",
                    arg.expression
                );
            }
            expression
        }
    };
    bits(result_width, expression)
}

fn evaluate_nary(
    op: NaryOp,
    operands: &[&SymbolicBits],
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
    if result_width == 0 {
        return bits(0, String::new());
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
    let operator = match op {
        NaryOp::And => "bvand",
        NaryOp::Or => "bvor",
        NaryOp::Xor => "bvxor",
        NaryOp::Concat => "concat",
        _ => return Err(symex_error(format!("unsupported n-ary operation {op:?}"))),
    };
    if op != NaryOp::Concat
        && operands
            .iter()
            .any(|operand| operand.bit_count != result_width)
    {
        return Err(symex_error("n-ary operand widths do not match result"));
    }
    let expression = operands
        .iter()
        .skip(1)
        .fold(operands[0].expression.clone(), |expression, operand| {
            format!("({operator} {expression} {})", operand.expression)
        });
    bits(result_width, expression)
}

fn select_structural(
    selector: &SymbolicBits,
    cases: &[SymbolicValue],
    default: Option<&SymbolicValue>,
) -> Result<SymbolicValue, XlsynthError> {
    let first = cases
        .first()
        .ok_or_else(|| symex_error("select has no cases"))?;
    match first {
        SymbolicValue::Bits(first_bits) => {
            let case_bits = cases
                .iter()
                .map(|case| match case {
                    SymbolicValue::Bits(bits) => Ok(bits),
                    _ => Err(symex_error("select cases have different structures")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let default_bits = match default {
                Some(SymbolicValue::Bits(bits)) => Some(bits),
                Some(_) => return Err(symex_error("select default has different structure")),
                None => None,
            };
            Ok(SymbolicValue::Bits(evaluate_sel(
                selector,
                &case_bits,
                default_bits,
                first_bits.bit_count,
            )?))
        }
        SymbolicValue::Tuple(first_elements) => {
            let mut result = Vec::with_capacity(first_elements.len());
            for index in 0..first_elements.len() {
                let element_cases = cases
                    .iter()
                    .map(|case| match case {
                        SymbolicValue::Tuple(elements) => elements
                            .get(index)
                            .cloned()
                            .ok_or_else(|| symex_error("select tuple arity mismatch")),
                        _ => Err(symex_error("select cases have different structures")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_element = match default {
                    Some(SymbolicValue::Tuple(elements)) => elements.get(index),
                    Some(_) => return Err(symex_error("select default has different structure")),
                    None => None,
                };
                result.push(select_structural(
                    selector,
                    &element_cases,
                    default_element,
                )?);
            }
            Ok(SymbolicValue::Tuple(result))
        }
        SymbolicValue::Array(first_elements) => {
            let mut result = Vec::with_capacity(first_elements.len());
            for index in 0..first_elements.len() {
                let element_cases = cases
                    .iter()
                    .map(|case| match case {
                        SymbolicValue::Array(elements) => elements
                            .get(index)
                            .cloned()
                            .ok_or_else(|| symex_error("select array length mismatch")),
                        _ => Err(symex_error("select cases have different structures")),
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let default_element = match default {
                    Some(SymbolicValue::Array(elements)) => elements.get(index),
                    Some(_) => return Err(symex_error("select default has different structure")),
                    None => None,
                };
                result.push(select_structural(
                    selector,
                    &element_cases,
                    default_element,
                )?);
            }
            Ok(SymbolicValue::Array(result))
        }
    }
}

fn evaluate_sel(
    selector: &SymbolicBits,
    cases: &[&SymbolicBits],
    default: Option<&SymbolicBits>,
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
    if result_width == 0 {
        return bits(0, String::new());
    }
    if cases.is_empty() {
        return Err(symex_error("select has no cases"));
    }
    if cases.iter().any(|case| case.bit_count != result_width)
        || default.is_some_and(|value| value.bit_count != result_width)
    {
        return Err(symex_error("select case widths do not match result"));
    }
    let fallback = default.unwrap_or(cases.last().unwrap()).expression.clone();
    let expression = cases
        .iter()
        .enumerate()
        .rev()
        .fold(fallback, |otherwise, (index, case)| {
            format!(
                "(ite (= {} (_ bv{index} {})) {} {otherwise})",
                selector.expression, selector.bit_count, case.expression
            )
        });
    bits(result_width, expression)
}

fn extend(
    operation: &str,
    arg: &SymbolicBits,
    new_bit_count: usize,
) -> Result<SymbolicBits, XlsynthError> {
    if new_bit_count < arg.bit_count {
        return Err(symex_error("extension cannot narrow its operand"));
    }
    let amount = new_bit_count - arg.bit_count;
    let expression = if new_bit_count == 0 {
        String::new()
    } else if arg.bit_count == 0 {
        format!("(_ bv0 {new_bit_count})")
    } else if amount == 0 {
        arg.expression.clone()
    } else {
        format!("((_ {operation} {amount}) {})", arg.expression)
    };
    bits(new_bit_count, expression)
}

fn binary(operation: &str, lhs: &SymbolicBits, rhs: &SymbolicBits) -> Result<String, XlsynthError> {
    equal_widths(lhs, rhs)?;
    Ok(format!(
        "({operation} {} {})",
        lhs.expression, rhs.expression
    ))
}

fn comparison(
    operation: &str,
    lhs: &SymbolicBits,
    rhs: &SymbolicBits,
) -> Result<String, XlsynthError> {
    equal_widths(lhs, rhs)?;
    Ok(format!(
        "(ite ({operation} {} {}) #b1 #b0)",
        lhs.expression, rhs.expression
    ))
}

fn shift(operation: &str, lhs: &SymbolicBits, rhs: &SymbolicBits) -> Result<String, XlsynthError> {
    let rhs = resize_unsigned(rhs, lhs.bit_count)?;
    Ok(format!(
        "({operation} {} {})",
        lhs.expression, rhs.expression
    ))
}

fn resize_unsigned(value: &SymbolicBits, width: usize) -> Result<SymbolicBits, XlsynthError> {
    if value.bit_count == width {
        return Ok(value.clone());
    }
    if value.bit_count < width {
        return extend("zero_extend", value, width);
    }
    bits(
        width,
        format!("((_ extract {} 0) {})", width - 1, value.expression),
    )
}

fn resize_signed(value: &SymbolicBits, width: usize) -> Result<SymbolicBits, XlsynthError> {
    if value.bit_count == width {
        return Ok(value.clone());
    }
    if value.bit_count < width {
        return extend("sign_extend", value, width);
    }
    bits(
        width,
        format!("((_ extract {} 0) {})", width - 1, value.expression),
    )
}

fn equal_widths(lhs: &SymbolicBits, rhs: &SymbolicBits) -> Result<(), XlsynthError> {
    if lhs.bit_count == rhs.bit_count {
        Ok(())
    } else {
        Err(symex_error(format!(
            "operand width mismatch: {} and {}",
            lhs.bit_count, rhs.bit_count
        )))
    }
}

fn get_value(
    values: &[Option<SymbolicValue>],
    node_ref: NodeRef,
) -> Result<&SymbolicValue, XlsynthError> {
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

fn get_bits(
    values: &[Option<SymbolicValue>],
    node_ref: NodeRef,
) -> Result<&SymbolicBits, XlsynthError> {
    match get_value(values, node_ref)? {
        SymbolicValue::Bits(bits) => Ok(bits),
        SymbolicValue::Tuple(_) | SymbolicValue::Array(_) => Err(symex_error(format!(
            "operand node {} is structured, expected bits",
            node_ref.index
        ))),
    }
}

fn symbolic_input(
    ty: &Type,
    name: &str,
    parameters: &mut Vec<SymbolicParameter>,
) -> Result<SymbolicValue, XlsynthError> {
    match ty {
        Type::Bits(0) => Ok(SymbolicValue::Bits(bits(0, String::new())?)),
        Type::Bits(bit_count) => {
            parameters.push(SymbolicParameter {
                name: name.to_owned(),
                bit_count: *bit_count,
            });
            Ok(SymbolicValue::Bits(bits(*bit_count, name.to_owned())?))
        }
        Type::Tuple(element_types) => Ok(SymbolicValue::Tuple(
            element_types
                .iter()
                .enumerate()
                .map(|(index, element_type)| {
                    symbolic_input(element_type, &format!("{name}_{index}"), parameters)
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Array(array) => Ok(SymbolicValue::Array(
            (0..array.element_count)
                .map(|index| {
                    symbolic_input(&array.element_type, &format!("{name}_{index}"), parameters)
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Type::Token => Err(symex_error("token parameters are not supported")),
    }
}

fn literal_value(ty: &Type, value: &IrValue) -> Result<SymbolicValue, XlsynthError> {
    match ty {
        Type::Bits(0) => Ok(SymbolicValue::Bits(bits(0, String::new())?)),
        Type::Bits(bit_count) => {
            let value = value.to_u64()?;
            Ok(SymbolicValue::Bits(bits(
                *bit_count,
                format!("(_ bv{value} {bit_count})"),
            )?))
        }
        Type::Tuple(element_types) => {
            let elements = value.get_elements()?;
            if elements.len() != element_types.len() {
                return Err(symex_error("tuple literal arity mismatch"));
            }
            Ok(SymbolicValue::Tuple(
                element_types
                    .iter()
                    .zip(&elements)
                    .map(|(element_type, element)| literal_value(element_type, element))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        Type::Array(array) => {
            let elements = value.get_elements()?;
            if elements.len() != array.element_count {
                return Err(symex_error("array literal length mismatch"));
            }
            Ok(SymbolicValue::Array(
                elements
                    .iter()
                    .map(|element| literal_value(&array.element_type, element))
                    .collect::<Result<Vec<_>, _>>()?,
            ))
        }
        Type::Token => Err(symex_error("token literals are not supported")),
    }
}

fn bits(bit_count: usize, expression: String) -> Result<SymbolicBits, XlsynthError> {
    Ok(SymbolicBits {
        bit_count,
        expression,
    })
}

fn bits_width(ty: &Type) -> Result<usize, XlsynthError> {
    match ty {
        Type::Bits(bit_count) => Ok(*bit_count),
        _ => Err(symex_error(format!(
            "unsupported symbolic value type: {ty}"
        ))),
    }
}

fn symex_error(message: impl Into<String>) -> XlsynthError {
    XlsynthError(format!("xlsynth-symex: {}", message.into()))
}
