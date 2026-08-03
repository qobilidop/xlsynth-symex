// SPDX-License-Identifier: Apache-2.0

//! Backend-neutral, typed, interned symbolic expressions.

use std::collections::HashMap;

use xlsynth::IrBits;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct ExprId(usize);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Sort {
    Bool,
    Bits(usize),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BitUnaryOp {
    Not,
    Neg,
}

impl BitUnaryOp {
    const fn smt_name(self) -> &'static str {
        match self {
            Self::Not => "bvnot",
            Self::Neg => "bvneg",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum BitBinaryOp {
    Add,
    Sub,
    Mul,
    Udiv,
    Sdiv,
    Urem,
    Srem,
    And,
    Or,
    Xor,
    Shl,
    Lshr,
    Ashr,
}

impl BitBinaryOp {
    const fn smt_name(self) -> &'static str {
        match self {
            Self::Add => "bvadd",
            Self::Sub => "bvsub",
            Self::Mul => "bvmul",
            Self::Udiv => "bvudiv",
            Self::Sdiv => "bvsdiv",
            Self::Urem => "bvurem",
            Self::Srem => "bvsrem",
            Self::And => "bvand",
            Self::Or => "bvor",
            Self::Xor => "bvxor",
            Self::Shl => "bvshl",
            Self::Lshr => "bvlshr",
            Self::Ashr => "bvashr",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CompareOp {
    Eq,
    Ult,
    Ule,
    Ugt,
    Uge,
    Slt,
    Sle,
    Sgt,
    Sge,
}

impl CompareOp {
    const fn smt_name(self) -> &'static str {
        match self {
            Self::Eq => "=",
            Self::Ult => "bvult",
            Self::Ule => "bvule",
            Self::Ugt => "bvugt",
            Self::Uge => "bvuge",
            Self::Slt => "bvslt",
            Self::Sle => "bvsle",
            Self::Sgt => "bvsgt",
            Self::Sge => "bvsge",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum ExprKind {
    BoolConst(bool),
    BitsConst(Vec<u8>),
    Variable(String),
    BoolNot(ExprId),
    BoolAnd(Vec<ExprId>),
    BoolOr(Vec<ExprId>),
    BitUnary(BitUnaryOp, ExprId),
    BitBinary(BitBinaryOp, ExprId, ExprId),
    Compare(CompareOp, ExprId, ExprId),
    Ite(ExprId, ExprId, ExprId),
    Concat(Vec<ExprId>),
    Extract {
        arg: ExprId,
        start: usize,
        width: usize,
    },
    Extend {
        arg: ExprId,
        signed: bool,
        amount: usize,
    },
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ExprNode {
    sort: Sort,
    kind: ExprKind,
}

#[derive(Default)]
pub(crate) struct ExprArena {
    nodes: Vec<ExprNode>,
    interned: HashMap<ExprNode, ExprId>,
}

impl ExprArena {
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub(crate) fn sort(&self, id: ExprId) -> Sort {
        self.nodes[id.0].sort
    }

    pub(crate) fn bool_const(&mut self, value: bool) -> ExprId {
        self.intern(ExprNode {
            sort: Sort::Bool,
            kind: ExprKind::BoolConst(value),
        })
    }

    pub(crate) fn bits_const(&mut self, bits: &IrBits) -> ExprId {
        let width = bits.get_bit_count();
        assert!(width > 0, "bits[0] has no SMT expression");
        self.intern(ExprNode {
            sort: Sort::Bits(width),
            kind: ExprKind::BitsConst(
                bits.to_le_bytes()
                    .expect("XLS bits must convert to little-endian bytes"),
            ),
        })
    }

    pub(crate) fn bits_const_u64(&mut self, width: usize, value: u64) -> ExprId {
        let bits = IrBits::make_ubits(width, value).expect("constant must fit declared width");
        self.bits_const(&bits)
    }

    pub(crate) fn variable(&mut self, name: impl Into<String>, sort: Sort) -> ExprId {
        assert_ne!(sort, Sort::Bits(0), "bits[0] has no SMT expression");
        self.intern(ExprNode {
            sort,
            kind: ExprKind::Variable(name.into()),
        })
    }

    pub(crate) fn bool_not(&mut self, arg: ExprId) -> ExprId {
        self.expect_sort(arg, Sort::Bool);
        match self.nodes[arg.0].kind {
            ExprKind::BoolConst(value) => self.bool_const(!value),
            ExprKind::BoolNot(inner) => inner,
            _ => self.intern(ExprNode {
                sort: Sort::Bool,
                kind: ExprKind::BoolNot(arg),
            }),
        }
    }

    pub(crate) fn bool_and(&mut self, args: impl IntoIterator<Item = ExprId>) -> ExprId {
        let mut flattened = Vec::new();
        for arg in args {
            self.expect_sort(arg, Sort::Bool);
            match &self.nodes[arg.0].kind {
                ExprKind::BoolConst(false) => return self.bool_const(false),
                ExprKind::BoolConst(true) => {}
                ExprKind::BoolAnd(nested) => flattened.extend(nested.iter().copied()),
                _ => flattened.push(arg),
            }
        }
        flattened.sort_unstable();
        flattened.dedup();
        match flattened.as_slice() {
            [] => self.bool_const(true),
            [only] => *only,
            _ => self.intern(ExprNode {
                sort: Sort::Bool,
                kind: ExprKind::BoolAnd(flattened),
            }),
        }
    }

    pub(crate) fn bool_or(&mut self, args: impl IntoIterator<Item = ExprId>) -> ExprId {
        let mut flattened = Vec::new();
        for arg in args {
            self.expect_sort(arg, Sort::Bool);
            match &self.nodes[arg.0].kind {
                ExprKind::BoolConst(true) => return self.bool_const(true),
                ExprKind::BoolConst(false) => {}
                ExprKind::BoolOr(nested) => flattened.extend(nested.iter().copied()),
                _ => flattened.push(arg),
            }
        }
        flattened.sort_unstable();
        flattened.dedup();
        match flattened.as_slice() {
            [] => self.bool_const(false),
            [only] => *only,
            _ => self.intern(ExprNode {
                sort: Sort::Bool,
                kind: ExprKind::BoolOr(flattened),
            }),
        }
    }

    pub(crate) fn bit_unary(&mut self, op: BitUnaryOp, arg: ExprId) -> ExprId {
        let Sort::Bits(width) = self.sort(arg) else {
            panic!("bit-vector unary operand must be bits-typed");
        };
        if let Some(bits) = self.bits_value(arg) {
            let result = match op {
                BitUnaryOp::Not => bits.not(),
                BitUnaryOp::Neg => bits.negate(),
            };
            return self.bits_const(&result);
        }
        self.intern(ExprNode {
            sort: Sort::Bits(width),
            kind: ExprKind::BitUnary(op, arg),
        })
    }

    pub(crate) fn bit_binary(&mut self, op: BitBinaryOp, lhs: ExprId, rhs: ExprId) -> ExprId {
        let sort = self.sort(lhs);
        self.expect_sort(rhs, sort);
        assert!(matches!(sort, Sort::Bits(_)));
        if let (Some(lhs_bits), Some(rhs_bits)) = (self.bits_value(lhs), self.bits_value(rhs)) {
            let result = match op {
                BitBinaryOp::Add => lhs_bits.add(&rhs_bits),
                BitBinaryOp::Sub => lhs_bits.sub(&rhs_bits),
                BitBinaryOp::Mul => lhs_bits.umul(&rhs_bits),
                BitBinaryOp::Udiv => lhs_bits.udiv(&rhs_bits),
                BitBinaryOp::Sdiv => lhs_bits.sdiv(&rhs_bits),
                BitBinaryOp::Urem => lhs_bits.umod(&rhs_bits),
                BitBinaryOp::Srem => lhs_bits.smod(&rhs_bits),
                BitBinaryOp::And => lhs_bits.and(&rhs_bits),
                BitBinaryOp::Or => lhs_bits.or(&rhs_bits),
                BitBinaryOp::Xor => lhs_bits.xor(&rhs_bits),
                BitBinaryOp::Shl => constant_shift(&lhs_bits, &rhs_bits, BitBinaryOp::Shl),
                BitBinaryOp::Lshr => constant_shift(&lhs_bits, &rhs_bits, BitBinaryOp::Lshr),
                BitBinaryOp::Ashr => constant_shift(&lhs_bits, &rhs_bits, BitBinaryOp::Ashr),
            };
            let result = resize_low_bits(
                &result,
                match sort {
                    Sort::Bits(width) => width,
                    Sort::Bool => unreachable!(),
                },
                false,
            );
            return self.bits_const(&result);
        }
        self.intern(ExprNode {
            sort,
            kind: ExprKind::BitBinary(op, lhs, rhs),
        })
    }

    pub(crate) fn compare(&mut self, op: CompareOp, lhs: ExprId, rhs: ExprId) -> ExprId {
        let sort = self.sort(lhs);
        self.expect_sort(rhs, sort);
        assert!(matches!(sort, Sort::Bits(_)));
        if lhs == rhs {
            return self.bool_const(matches!(
                op,
                CompareOp::Eq | CompareOp::Ule | CompareOp::Uge | CompareOp::Sle | CompareOp::Sge
            ));
        }
        if let (Some(lhs_bits), Some(rhs_bits)) = (self.bits_value(lhs), self.bits_value(rhs)) {
            let value = match op {
                CompareOp::Eq => lhs_bits.equals(&rhs_bits),
                CompareOp::Ult => lhs_bits.ult(&rhs_bits),
                CompareOp::Ule => lhs_bits.ule(&rhs_bits),
                CompareOp::Ugt => lhs_bits.ugt(&rhs_bits),
                CompareOp::Uge => lhs_bits.uge(&rhs_bits),
                CompareOp::Slt => lhs_bits.slt(&rhs_bits),
                CompareOp::Sle => lhs_bits.sle(&rhs_bits),
                CompareOp::Sgt => lhs_bits.sgt(&rhs_bits),
                CompareOp::Sge => lhs_bits.sge(&rhs_bits),
            };
            return self.bool_const(value);
        }
        self.intern(ExprNode {
            sort: Sort::Bool,
            kind: ExprKind::Compare(op, lhs, rhs),
        })
    }

    pub(crate) fn ite(&mut self, condition: ExprId, then_id: ExprId, else_id: ExprId) -> ExprId {
        self.expect_sort(condition, Sort::Bool);
        let sort = self.sort(then_id);
        self.expect_sort(else_id, sort);
        if then_id == else_id {
            return then_id;
        }
        match self.nodes[condition.0].kind {
            ExprKind::BoolConst(true) => then_id,
            ExprKind::BoolConst(false) => else_id,
            _ => self.intern(ExprNode {
                sort,
                kind: ExprKind::Ite(condition, then_id, else_id),
            }),
        }
    }

    pub(crate) fn bool_to_bit(&mut self, condition: ExprId) -> ExprId {
        let one = self.bits_const_u64(1, 1);
        let zero = self.bits_const_u64(1, 0);
        self.ite(condition, one, zero)
    }

    pub(crate) fn bit_is_one(&mut self, bit: ExprId) -> ExprId {
        self.expect_sort(bit, Sort::Bits(1));
        let one = self.bits_const_u64(1, 1);
        self.compare(CompareOp::Eq, bit, one)
    }

    pub(crate) fn concat(&mut self, args: Vec<ExprId>) -> ExprId {
        assert!(!args.is_empty());
        let width = args
            .iter()
            .map(|arg| match self.sort(*arg) {
                Sort::Bits(width) => width,
                Sort::Bool => panic!("concat operand must be bits-typed"),
            })
            .sum();
        match args.as_slice() {
            [only] => *only,
            _ => {
                if let Some(constants) = args
                    .iter()
                    .map(|arg| self.bits_value(*arg))
                    .collect::<Option<Vec<_>>>()
                {
                    let mut output = Vec::with_capacity(width);
                    for value in constants.iter().rev() {
                        for index in 0..value.get_bit_count() {
                            output.push(value.get_bit(index).expect("bit index must be valid"));
                        }
                    }
                    return self.bits_const(&IrBits::from_lsb_is_0(&output));
                }
                self.intern(ExprNode {
                    sort: Sort::Bits(width),
                    kind: ExprKind::Concat(args),
                })
            }
        }
    }

    pub(crate) fn extract(&mut self, arg: ExprId, start: usize, width: usize) -> ExprId {
        let Sort::Bits(arg_width) = self.sort(arg) else {
            panic!("extract operand must be bits-typed");
        };
        assert!(width > 0 && start + width <= arg_width);
        if start == 0 && width == arg_width {
            return arg;
        }
        if let Some(bits) = self.bits_value(arg) {
            let extracted = bits.width_slice(start as i64, width as i64);
            return self.bits_const(&extracted);
        }
        self.intern(ExprNode {
            sort: Sort::Bits(width),
            kind: ExprKind::Extract { arg, start, width },
        })
    }

    pub(crate) fn extend(&mut self, arg: ExprId, new_width: usize, signed: bool) -> ExprId {
        let Sort::Bits(old_width) = self.sort(arg) else {
            panic!("extension operand must be bits-typed");
        };
        assert!(new_width >= old_width);
        let amount = new_width - old_width;
        if amount == 0 {
            return arg;
        }
        if let Some(bits) = self.bits_value(arg) {
            return self.bits_const(&resize_low_bits(&bits, new_width, signed));
        }
        self.intern(ExprNode {
            sort: Sort::Bits(new_width),
            kind: ExprKind::Extend {
                arg,
                signed,
                amount,
            },
        })
    }

    pub(crate) fn to_smtlib(&self, id: ExprId) -> String {
        match &self.nodes[id.0].kind {
            ExprKind::BoolConst(value) => value.to_string(),
            ExprKind::BitsConst(bytes) => {
                let Sort::Bits(width) = self.sort(id) else {
                    unreachable!()
                };
                let mut result = String::with_capacity(2 + width);
                result.push_str("#b");
                for index in (0..width).rev() {
                    result.push(if bytes[index / 8] & (1 << (index % 8)) == 0 {
                        '0'
                    } else {
                        '1'
                    });
                }
                result
            }
            ExprKind::Variable(name) => name.clone(),
            ExprKind::BoolNot(arg) => format!("(not {})", self.to_smtlib(*arg)),
            ExprKind::BoolAnd(args) => self.render_nary("and", args),
            ExprKind::BoolOr(args) => self.render_nary("or", args),
            ExprKind::BitUnary(op, arg) => {
                format!("({} {})", op.smt_name(), self.to_smtlib(*arg))
            }
            ExprKind::BitBinary(op, lhs, rhs) => format!(
                "({} {} {})",
                op.smt_name(),
                self.to_smtlib(*lhs),
                self.to_smtlib(*rhs)
            ),
            ExprKind::Compare(op, lhs, rhs) => format!(
                "({} {} {})",
                op.smt_name(),
                self.to_smtlib(*lhs),
                self.to_smtlib(*rhs)
            ),
            ExprKind::Ite(condition, then_id, else_id) => format!(
                "(ite {} {} {})",
                self.to_smtlib(*condition),
                self.to_smtlib(*then_id),
                self.to_smtlib(*else_id)
            ),
            ExprKind::Concat(args) => self.render_nary("concat", args),
            ExprKind::Extract { arg, start, width } => format!(
                "((_ extract {} {}) {})",
                start + width - 1,
                start,
                self.to_smtlib(*arg)
            ),
            ExprKind::Extend {
                arg,
                signed,
                amount,
            } => format!(
                "((_ {} {amount}) {})",
                if *signed {
                    "sign_extend"
                } else {
                    "zero_extend"
                },
                self.to_smtlib(*arg)
            ),
        }
    }

    pub(crate) fn bool_value(&self, id: ExprId) -> Option<bool> {
        match self.nodes[id.0].kind {
            ExprKind::BoolConst(value) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn bits_value(&self, id: ExprId) -> Option<IrBits> {
        let ExprKind::BitsConst(bytes) = &self.nodes[id.0].kind else {
            return None;
        };
        let Sort::Bits(width) = self.sort(id) else {
            return None;
        };
        IrBits::from_le_bytes(width, bytes).ok()
    }

    fn render_nary(&self, operator: &str, args: &[ExprId]) -> String {
        let mut rendered = args.iter().map(|arg| self.to_smtlib(*arg));
        let first = rendered
            .next()
            .expect("n-ary expression must have operands");
        rendered.fold(first, |lhs, rhs| format!("({operator} {lhs} {rhs})"))
    }

    fn expect_sort(&self, id: ExprId, expected: Sort) {
        assert_eq!(self.sort(id), expected, "symbolic expression sort mismatch");
    }

    fn intern(&mut self, node: ExprNode) -> ExprId {
        if let Some(id) = self.interned.get(&node) {
            return *id;
        }
        let id = ExprId(self.nodes.len());
        self.nodes.push(node.clone());
        self.interned.insert(node, id);
        id
    }
}

fn resize_low_bits(bits: &IrBits, width: usize, signed: bool) -> IrBits {
    let old_width = bits.get_bit_count();
    let sign = signed && old_width > 0 && bits.get_bit(old_width - 1).unwrap();
    let output = (0..width)
        .map(|index| {
            if index < old_width {
                bits.get_bit(index).unwrap()
            } else {
                sign
            }
        })
        .collect::<Vec<_>>();
    IrBits::from_lsb_is_0(&output)
}

fn constant_shift(lhs: &IrBits, rhs: &IrBits, operation: BitBinaryOp) -> IrBits {
    let width = lhs.get_bit_count();
    let shift = rhs
        .to_u64()
        .ok()
        .and_then(|value| usize::try_from(value).ok());
    let Some(shift) = shift.filter(|shift| *shift < width) else {
        let fill = operation == BitBinaryOp::Ashr && width > 0 && lhs.get_bit(width - 1).unwrap();
        return IrBits::from_lsb_is_0(&vec![fill; width]);
    };
    match operation {
        BitBinaryOp::Shl => lhs.shll(shift as i64),
        BitBinaryOp::Lshr => lhs.shrl(shift as i64),
        BitBinaryOp::Ashr => lhs.shra(shift as i64),
        _ => unreachable!("constant_shift requires a shift operation"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interns_and_renders_typed_expressions() {
        let mut arena = ExprArena::default();
        let x = arena.variable("x", Sort::Bits(8));
        let y = arena.variable("y", Sort::Bits(8));
        let sum = arena.bit_binary(BitBinaryOp::Add, x, y);
        let repeated = arena.bit_binary(BitBinaryOp::Add, x, y);
        assert_eq!(sum, repeated);
        assert_eq!(arena.to_smtlib(sum), "(bvadd x y)");

        let three = arena.bits_const_u64(8, 3);
        let equal = arena.compare(CompareOp::Eq, sum, three);
        assert_eq!(arena.to_smtlib(equal), "(= (bvadd x y) #b00000011)");
    }

    #[test]
    fn simplifies_boolean_path_conditions() {
        let mut arena = ExprArena::default();
        let condition = arena.variable("condition", Sort::Bool);
        let true_id = arena.bool_const(true);
        let combined = arena.bool_and([true_id, condition, condition]);
        assert_eq!(combined, condition);
        let negated = arena.bool_not(condition);
        assert_eq!(arena.to_smtlib(negated), "(not condition)");
    }

    #[test]
    fn renders_arbitrary_width_constants() {
        let mut arena = ExprArena::default();
        let bits = IrBits::from_msb_is_0(&[
            true, false, true, false, true, false, true, false, true, false,
        ]);
        let id = arena.bits_const(&bits);
        assert_eq!(arena.to_smtlib(id), "#b1010101010");
    }
}
