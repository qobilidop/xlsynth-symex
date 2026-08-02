// SPDX-License-Identifier: Apache-2.0

use xlsynth::XlsynthError;
use xlsynth_pir::ir::{Binop, Fn, NaryOp, NodePayload, NodeRef, Package, Type, Unop};
use xlsynth_pir::ir_parser::Parser;

use crate::{PathCondition, SymbolicBits, SymbolicParameter, SymexResult};

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
    let parameters = function
        .params
        .iter()
        .enumerate()
        .map(|(index, parameter)| match parameter.ty {
            Type::Bits(bit_count) if bit_count > 0 => Ok(SymbolicParameter {
                name: format!("symex_arg_{index}"),
                bit_count,
            }),
            ref ty => Err(symex_error(format!(
                "unsupported parameter type for {}: {ty}",
                parameter.name
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let arguments = parameters
        .iter()
        .map(|parameter| SymbolicBits {
            bit_count: parameter.bit_count,
            expression: parameter.name.clone(),
        })
        .collect();
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
    result_smtlib.push_str(&format!(
        "(define-fun xlsynth_symex_result () (_ BitVec {}) {})\n",
        result.bit_count, result.expression
    ));
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
        arguments: Vec<SymbolicBits>,
    ) -> Result<SymbolicBits, XlsynthError> {
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
                    let bit_count = bits_width(&node.ty)?;
                    let value = value.to_u64().map_err(|error| {
                        symex_error(format!(
                            "unsupported literal at node {}: {error}",
                            node.text_id
                        ))
                    })?;
                    bits(bit_count, format!("(_ bv{value} {bit_count})"))?
                }
                NodePayload::Binop(op, lhs, rhs) => {
                    let lhs = get_value(&values, *lhs)?;
                    let rhs = get_value(&values, *rhs)?;
                    evaluate_binop(*op, lhs, rhs, bits_width(&node.ty)?)?
                }
                NodePayload::Unop(op, arg) => {
                    evaluate_unop(*op, get_value(&values, *arg)?, bits_width(&node.ty)?)?
                }
                NodePayload::Nary(op, operands) => {
                    let operands = operands
                        .iter()
                        .map(|operand| get_value(&values, *operand))
                        .collect::<Result<Vec<_>, _>>()?;
                    evaluate_nary(*op, &operands, bits_width(&node.ty)?)?
                }
                NodePayload::ZeroExt { arg, new_bit_count } => {
                    let arg = get_value(&values, *arg)?;
                    extend("zero_extend", arg, *new_bit_count)?
                }
                NodePayload::SignExt { arg, new_bit_count } => {
                    let arg = get_value(&values, *arg)?;
                    extend("sign_extend", arg, *new_bit_count)?
                }
                NodePayload::BitSlice { arg, start, width } => {
                    let arg = get_value(&values, *arg)?;
                    if *width == 0 {
                        return Err(symex_error("zero-width bit slices are not yet supported"));
                    }
                    bits(
                        *width,
                        format!(
                            "((_ extract {} {}) {})",
                            start + width - 1,
                            start,
                            arg.expression
                        ),
                    )?
                }
                NodePayload::DynamicBitSlice { arg, start, width } => {
                    let arg = get_value(&values, *arg)?;
                    let start = resize_unsigned(get_value(&values, *start)?, arg.bit_count)?;
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
                    bits(*width, expression)?
                }
                NodePayload::Sel {
                    selector,
                    cases,
                    default,
                } => {
                    let selector = get_value(&values, *selector)?;
                    let cases = cases
                        .iter()
                        .map(|case| get_value(&values, *case))
                        .collect::<Result<Vec<_>, _>>()?;
                    let default = default.map(|value| get_value(&values, value)).transpose()?;
                    evaluate_sel(selector, &cases, default, bits_width(&node.ty)?)?
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

fn evaluate_binop(
    op: Binop,
    lhs: &SymbolicBits,
    rhs: &SymbolicBits,
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
    let expression = match op {
        Binop::Add => binary("bvadd", lhs, rhs)?,
        Binop::Sub => binary("bvsub", lhs, rhs)?,
        Binop::Umul | Binop::Smul => binary("bvmul", lhs, rhs)?,
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
    let expression = match op {
        Unop::Identity => arg.expression.clone(),
        Unop::Neg => format!("(bvneg {})", arg.expression),
        Unop::Not => format!("(bvnot {})", arg.expression),
        _ => return Err(symex_error(format!("unsupported unary operation {op:?}"))),
    };
    bits(result_width, expression)
}

fn evaluate_nary(
    op: NaryOp,
    operands: &[&SymbolicBits],
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
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

fn evaluate_sel(
    selector: &SymbolicBits,
    cases: &[&SymbolicBits],
    default: Option<&SymbolicBits>,
    result_width: usize,
) -> Result<SymbolicBits, XlsynthError> {
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
    let expression = if amount == 0 {
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
    values: &[Option<SymbolicBits>],
    node_ref: NodeRef,
) -> Result<&SymbolicBits, XlsynthError> {
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

fn bits(bit_count: usize, expression: String) -> Result<SymbolicBits, XlsynthError> {
    if bit_count == 0 {
        Err(symex_error("zero-width bits are not yet supported"))
    } else {
        Ok(SymbolicBits {
            bit_count,
            expression,
        })
    }
}

fn bits_width(ty: &Type) -> Result<usize, XlsynthError> {
    match ty {
        Type::Bits(bit_count) if *bit_count > 0 => Ok(*bit_count),
        _ => Err(symex_error(format!(
            "unsupported symbolic value type: {ty}"
        ))),
    }
}

fn symex_error(message: impl Into<String>) -> XlsynthError {
    XlsynthError(format!("xlsynth-symex: {}", message.into()))
}
