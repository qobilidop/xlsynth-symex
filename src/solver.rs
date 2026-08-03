// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

use xlsynth::{IrBits, XlsynthError};

use crate::SymbolicParameter;

pub(crate) enum Satisfiability {
    Sat(BTreeMap<String, IrBits>),
    Unsat,
    Indeterminate(String),
}

pub(crate) fn solve(
    parameters: &[SymbolicParameter],
    condition: &str,
) -> Result<Satisfiability, XlsynthError> {
    let mut query = String::from("(set-option :produce-models true)\n");
    for parameter in parameters {
        query.push_str(&format!(
            "(declare-const {} (_ BitVec {}))\n",
            parameter.name, parameter.bit_count
        ));
    }
    query.push_str(&format!("(assert {condition})\n(check-sat)\n"));
    if !parameters.is_empty() {
        query.push_str("(get-value (");
        query.push_str(
            &parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>()
                .join(" "),
        );
        query.push_str("))\n");
    }

    let mut child = Command::new("z3")
        .arg("-in")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| XlsynthError(format!("xlsynth-symex: failed to start z3: {error}")))?;
    child
        .stdin
        .take()
        .expect("piped z3 stdin must be present")
        .write_all(query.as_bytes())
        .map_err(|error| {
            XlsynthError(format!("xlsynth-symex: failed to write z3 query: {error}"))
        })?;
    let output = child.wait_with_output().map_err(|error| {
        XlsynthError(format!(
            "xlsynth-symex: failed to collect z3 result: {error}"
        ))
    })?;
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| XlsynthError(format!("xlsynth-symex: z3 output is not UTF-8: {error}")))?;
    let expressions = parse_sexpressions(&stdout)?;
    let Some(status) = expressions.first().and_then(Sexp::as_atom) else {
        return Ok(Satisfiability::Indeterminate(format!(
            "z3 returned no status: {stdout:?}"
        )));
    };
    match status {
        "unsat" => Ok(Satisfiability::Unsat),
        "unknown" => Ok(Satisfiability::Indeterminate(
            "z3 returned unknown".to_owned(),
        )),
        "sat" if !output.status.success() => Ok(Satisfiability::Indeterminate(format!(
            "z3 exited with {} after reporting sat: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        "sat" => {
            if parameters.is_empty() {
                return Ok(Satisfiability::Sat(BTreeMap::new()));
            }
            let Some(Sexp::List(bindings)) = expressions.get(1) else {
                return Ok(Satisfiability::Indeterminate(format!(
                    "z3 omitted model values: {stdout:?}"
                )));
            };
            let widths = parameters
                .iter()
                .map(|parameter| (parameter.name.as_str(), parameter.bit_count))
                .collect::<BTreeMap<_, _>>();
            let mut model = BTreeMap::new();
            for binding in bindings {
                let Sexp::List(pair) = binding else {
                    return model_error(&stdout, "model binding is not a list");
                };
                if pair.len() != 2 {
                    return model_error(&stdout, "model binding is not a pair");
                }
                let Some(name) = pair[0].as_atom() else {
                    return model_error(&stdout, "model binding name is not an atom");
                };
                let Some(width) = widths.get(name).copied() else {
                    return model_error(&stdout, "model contains an unknown parameter");
                };
                model.insert(name.to_owned(), parse_bit_vector(&pair[1], width)?);
            }
            if model.len() != parameters.len() {
                return model_error(&stdout, "model omitted a symbolic parameter");
            }
            Ok(Satisfiability::Sat(model))
        }
        other => Ok(Satisfiability::Indeterminate(format!(
            "unexpected z3 status {other:?}"
        ))),
    }
}

fn model_error<T>(stdout: &str, message: &str) -> Result<T, XlsynthError> {
    Err(XlsynthError(format!(
        "xlsynth-symex: {message} in z3 output {stdout:?}"
    )))
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Sexp {
    Atom(String),
    List(Vec<Sexp>),
}

impl Sexp {
    fn as_atom(&self) -> Option<&str> {
        match self {
            Self::Atom(atom) => Some(atom),
            Self::List(_) => None,
        }
    }
}

fn parse_sexpressions(text: &str) -> Result<Vec<Sexp>, XlsynthError> {
    let tokens = tokenize(text);
    let mut index = 0;
    let mut result = Vec::new();
    while index < tokens.len() {
        result.push(parse_one(&tokens, &mut index)?);
    }
    Ok(result)
}

fn tokenize(text: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character == '(' || character == ')' || character.is_whitespace() {
            if let Some(start) = start.take() {
                result.push(&text[start..index]);
            }
            if character == '(' || character == ')' {
                result.push(&text[index..index + 1]);
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(start) = start {
        result.push(&text[start..]);
    }
    result
}

fn parse_one(tokens: &[&str], index: &mut usize) -> Result<Sexp, XlsynthError> {
    let Some(token) = tokens.get(*index).copied() else {
        return Err(XlsynthError(
            "xlsynth-symex: unexpected end of solver S-expression".to_owned(),
        ));
    };
    *index += 1;
    if token == "(" {
        let mut children = Vec::new();
        while tokens.get(*index).copied() != Some(")") {
            children.push(parse_one(tokens, index)?);
        }
        *index += 1;
        Ok(Sexp::List(children))
    } else if token == ")" {
        Err(XlsynthError(
            "xlsynth-symex: unmatched ')' in solver output".to_owned(),
        ))
    } else {
        Ok(Sexp::Atom(token.to_owned()))
    }
}

fn parse_bit_vector(value: &Sexp, width: usize) -> Result<IrBits, XlsynthError> {
    if let Some(atom) = value.as_atom() {
        if let Some(binary) = atom.strip_prefix("#b") {
            return bits_from_radix(binary, 1, width);
        }
        if let Some(hex) = atom.strip_prefix("#x") {
            return bits_from_radix(hex, 4, width);
        }
    }
    if let Sexp::List(parts) = value
        && parts.len() == 3
        && parts[0].as_atom() == Some("_")
        && parts[2].as_atom() == Some(&width.to_string())
        && let Some(decimal) = parts[1].as_atom().and_then(|atom| atom.strip_prefix("bv"))
    {
        return bits_from_decimal(decimal, width);
    }
    Err(XlsynthError(format!(
        "xlsynth-symex: unsupported z3 bit-vector value {value:?} for bits[{width}]"
    )))
}

fn bits_from_radix(text: &str, digit_width: usize, width: usize) -> Result<IrBits, XlsynthError> {
    let mut bits = Vec::with_capacity(text.len() * digit_width);
    for character in text.chars().rev() {
        let digit = character.to_digit(1_u32 << digit_width).ok_or_else(|| {
            XlsynthError(format!("xlsynth-symex: invalid model digit {character:?}"))
        })?;
        for bit in 0..digit_width {
            bits.push(digit & (1 << bit) != 0);
        }
    }
    bits.resize(width, false);
    bits.truncate(width);
    Ok(IrBits::from_lsb_is_0(&bits))
}

fn bits_from_decimal(text: &str, width: usize) -> Result<IrBits, XlsynthError> {
    let mut bits = vec![false; width];
    for character in text.chars() {
        let digit = character.to_digit(10).ok_or_else(|| {
            XlsynthError(format!("xlsynth-symex: invalid model decimal {text:?}"))
        })?;
        let mut carry = digit;
        for bit in &mut bits {
            let value = u32::from(*bit) * 10 + carry;
            *bit = value & 1 != 0;
            carry = value >> 1;
        }
        if carry != 0 {
            return Err(XlsynthError(format!(
                "xlsynth-symex: model value {text} does not fit bits[{width}]"
            )));
        }
    }
    Ok(IrBits::from_lsb_is_0(&bits))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_z3_models_at_non_nibble_and_wide_widths() {
        let parameters = vec![
            SymbolicParameter {
                name: "x".to_owned(),
                bit_count: 5,
            },
            SymbolicParameter {
                name: "wide".to_owned(),
                bit_count: 80,
            },
        ];
        let Satisfiability::Sat(model) = solve(
            &parameters,
            "(and (= x #b10101) (= wide #x123456789abcdef01234))",
        )
        .unwrap() else {
            panic!("constraint must be satisfiable");
        };
        assert_eq!(model["x"].to_u64().unwrap(), 21);
        assert_eq!(model["wide"].get_bit_count(), 80);
        assert_eq!(
            model["wide"].to_string(),
            "bits[80]:0x1234_5678_9abc_def0_1234"
        );
    }

    #[test]
    fn reports_unsatisfiable_constraints() {
        let parameters = vec![SymbolicParameter {
            name: "x".to_owned(),
            bit_count: 1,
        }];
        assert!(matches!(
            solve(&parameters, "(and (= x #b0) (= x #b1))").unwrap(),
            Satisfiability::Unsat
        ));
    }
}
