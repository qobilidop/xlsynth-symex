// SPDX-License-Identifier: Apache-2.0

//! Checks the pinned XLS/PIR operation inventory used by the v1 release gate.

use std::collections::BTreeMap;

const MATRIX: &str = include_str!("../docs/user/support-matrix.md");
const OPERATION_TESTS: &str = include_str!("operation_semantics.rs");
const SELECTION_TESTS: &str = include_str!("selection_enumeration.rs");
const XLSYNTH_CRATE_REVISION: &str = "92bc9b932981c776bb4bb197cd6b6726f17ec090";

fn markdown_cells(line: &str) -> Vec<&str> {
    line.strip_prefix('|')
        .and_then(|line| line.strip_suffix('|'))
        .expect("support matrix table rows must begin and end with `|`")
        .split('|')
        .map(str::trim)
        .collect()
}

fn code_cell<'a>(cell: &'a str, line: &str) -> &'a str {
    let Some(value) = cell
        .strip_prefix('`')
        .and_then(|cell| cell.strip_suffix('`'))
    else {
        panic!("support matrix identifiers must be code-formatted: {line}");
    };
    assert!(!value.is_empty(), "empty code-formatted cell: {line}");
    value
}

#[test]
fn support_matrix_is_complete_and_well_formed() {
    assert!(
        MATRIX.contains(&format!("`{XLSYNTH_CRATE_REVISION}`.")),
        "support matrix must name the pinned xlsynth-crate revision"
    );

    let mut entries = BTreeMap::new();
    let mut current_status = None;
    for line in MATRIX.lines().map(str::trim) {
        if line.starts_with("## ") {
            current_status = match line {
                "## Supported operations" => Some("supported"),
                "## Excluded operations" => Some("excluded"),
                _ => None,
            };
            continue;
        }
        if !line.starts_with('|') || line.starts_with("|---") || line.starts_with("| Operation") {
            continue;
        }

        let status = current_status
            .unwrap_or_else(|| panic!("support matrix row outside a status section: {line}"));
        let fields = markdown_cells(line);
        let (operation, coverage) = if status == "supported" {
            assert_eq!(
                fields.len(),
                3,
                "supported rows need operation, semantics, and coverage cells: {line}"
            );
            let operation = code_cell(fields[0], line);
            assert!(
                !fields[1].is_empty(),
                "{operation} has no semantic rationale"
            );
            let coverage = code_cell(fields[2], line);
            let declaration = format!("fn {coverage}(");
            assert!(
                OPERATION_TESTS.contains(&declaration) || SELECTION_TESTS.contains(&declaration),
                "{operation} names unknown executable coverage target {coverage}"
            );
            (operation, Some(coverage))
        } else {
            assert_eq!(
                fields.len(),
                2,
                "excluded rows need operation and reason cells: {line}"
            );
            let operation = code_cell(fields[0], line);
            assert!(!fields[1].is_empty(), "{operation} has no exclusion reason");
            (operation, None)
        };
        assert!(
            entries.insert(operation, (status, coverage)).is_none(),
            "duplicate operation {operation}"
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
    assert_eq!(
        entries
            .values()
            .filter(|(status, _)| *status == "supported")
            .count(),
        65
    );
    assert_eq!(
        entries
            .values()
            .filter(|(status, _)| *status == "excluded")
            .count(),
        8
    );
    assert!(
        entries
            .iter()
            .all(|(_, (status, _))| matches!(*status, "supported" | "excluded")),
        "v1 operation matrix must not contain partial or gap rows: {entries:?}"
    );
}
