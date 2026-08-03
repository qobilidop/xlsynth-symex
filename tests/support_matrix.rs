// SPDX-License-Identifier: Apache-2.0

//! Checks the pinned XLS/PIR operation inventory used by the v1 release gate.

use std::collections::BTreeMap;

const MATRIX: &str = include_str!("../docs/support-matrix.tsv");
const OPERATION_TESTS: &str = include_str!("operation_semantics.rs");
const PATH_TESTS: &str = include_str!("path_enumeration.rs");

#[test]
fn support_matrix_is_complete_and_well_formed() {
    let mut entries = BTreeMap::new();
    for line in MATRIX
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
    {
        let fields = line.split('\t').collect::<Vec<_>>();
        assert_eq!(
            fields.len(),
            4,
            "support matrix row must have four tab-separated fields: {line}"
        );
        assert!(
            matches!(fields[1], "supported" | "partial" | "gap" | "excluded"),
            "{} has unknown status {}",
            fields[0],
            fields[1]
        );
        assert!(!fields[2].is_empty(), "{} has no rationale", fields[0]);
        let coverage = fields[3];
        if fields[1] == "supported" {
            let declaration = format!("fn {coverage}(");
            assert!(
                OPERATION_TESTS.contains(&declaration) || PATH_TESTS.contains(&declaration),
                "{} names unknown executable coverage target {coverage}",
                fields[0]
            );
        } else {
            assert_eq!(coverage, "n/a", "{} exclusion coverage", fields[0]);
        }
        assert!(
            entries.insert(fields[0], (fields[1], coverage)).is_none(),
            "duplicate operation {}",
            fields[0]
        );
    }

    let expected = [
        "add",
        "after_all",
        "and",
        "and_reduce",
        "array",
        "array_concat",
        "array_index",
        "array_slice",
        "array_update",
        "assert",
        "bit_slice",
        "bit_slice_update",
        "concat",
        "counted_for",
        "cover",
        "decode",
        "dynamic_bit_slice",
        "encode",
        "eq",
        "ext_carry_out",
        "ext_clz",
        "ext_mask_low",
        "ext_nary_add",
        "ext_normalize_left",
        "ext_prio_encode",
        "gate",
        "identity",
        "instantiation_input",
        "instantiation_output",
        "invoke",
        "literal",
        "nand",
        "ne",
        "neg",
        "nor",
        "not",
        "one_hot",
        "one_hot_sel",
        "or",
        "or_reduce",
        "param",
        "priority_sel",
        "register_read",
        "register_write",
        "reverse",
        "sdiv",
        "sel",
        "sge",
        "sgt",
        "shll",
        "shra",
        "shrl",
        "sign_ext",
        "sle",
        "slt",
        "smod",
        "smul",
        "smulp",
        "sub",
        "trace",
        "tuple",
        "tuple_index",
        "udiv",
        "uge",
        "ugt",
        "ule",
        "ult",
        "umod",
        "umul",
        "umulp",
        "xor",
        "xor_reduce",
        "zero_ext",
    ];
    assert_eq!(entries.keys().copied().collect::<Vec<_>>(), expected);
    assert!(
        entries
            .iter()
            .all(|(_, (status, _))| matches!(*status, "supported" | "excluded")),
        "v1 operation matrix must not contain partial or gap rows: {entries:?}"
    );
}
