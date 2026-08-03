# XLS operation support

This document is the authoritative operation inventory for the pure-function
domain supported by `xlsynth-symex`. Tests parse these tables, verify that every
supported row names an executable coverage target, and reject missing,
duplicated, partial, or unclassified operations.

Pinned `xlsynth-crate` revision:
`92bc9b932981c776bb4bb197cd6b6726f17ec090`.

## Supported operations

### Values and structure

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `param` | Symbolic bits, tuple, and array parameters | `arbitrary_width_parameters_and_literals_are_native_values` |
| `literal` | Structured and arbitrary-width bits values | `arbitrary_width_parameters_and_literals_are_native_values` |
| `identity` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `array` | Finite structural value | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `array_index` | Multidimensional indexing with XLS out-of-bounds clamping | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `array_concat` | Finite structural array concatenation | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `array_slice` | XLS clamped slicing, including oversized starts | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `array_update` | Multidimensional update with XLS out-of-bounds behavior | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `tuple` | Finite structural value | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |
| `tuple_index` | Finite structural value | `nested_array_tuple_index_concat_slice_and_update_operations_match_xls` |

### Logic and encoding

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `not` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `reverse` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `or_reduce` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `and_reduce` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `xor_reduce` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `and` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `nand` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `nor` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `or` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `xor` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `gate` | Deep zeroing for bits and structured values | `unary_nary_encoding_and_gate_operations_match_xls` |
| `one_hot` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `encode` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `decode` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |

### Arithmetic and comparison

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `neg` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `add` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `sub` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `umul` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `smul` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `udiv` | Includes XLS divide-by-zero semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `sdiv` | Includes XLS signed divide-by-zero semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `umod` | Includes XLS modulo-by-zero semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `smod` | Includes XLS signed remainder semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `umulp` | Partial products satisfy the XLS modular-sum relation | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `smulp` | Signed partial products satisfy the XLS modular-sum relation | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `eq` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `ne` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `ugt` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `uge` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `ult` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `ule` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `sgt` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `sge` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `slt` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `sle` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |

### Shifts, slicing, and extension

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `shll` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `shrl` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `shra` | Merged value semantics | `arithmetic_comparison_shift_and_partial_product_operations_match_xls` |
| `zero_ext` | Merged value semantics | `slicing_update_and_extension_operations_match_xls` |
| `sign_ext` | Merged value semantics | `slicing_update_and_extension_operations_match_xls` |
| `bit_slice` | Merged value semantics | `slicing_update_and_extension_operations_match_xls` |
| `concat` | Merged value semantics | `unary_nary_encoding_and_gate_operations_match_xls` |
| `dynamic_bit_slice` | Merged data selection | `slicing_update_and_extension_operations_match_xls` |
| `bit_slice_update` | Merged data selection, including oversized starts | `slicing_update_and_extension_operations_match_xls` |

### Selection enumeration

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `sel` | Unsigned-index case/default enumeration | `selector_encodings_interpret_the_same_bits_differently` |
| `priority_sel` | Complete lowest-index-priority case/default enumeration | `priority_and_one_hot_outcomes_follow_v1_policy` |
| `one_hot_sel` | Complete selected-bitmask enumeration with recursive OR semantics | `selector_encodings_interpret_the_same_bits_differently` |

### `xlsynth-pir` extensions

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `ext_carry_out` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |
| `ext_prio_encode` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |
| `ext_clz` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |
| `ext_normalize_left` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |
| `ext_mask_low` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |
| `ext_nary_add` | Desugared to pinned ordinary PIR operations | `pir_extension_operations_desugar_before_symbolic_evaluation` |

### Calls and bounded iteration

| Operation | Semantics and limitations | Executable coverage |
|---|---|---|
| `invoke` | Demand evaluation with callsite-qualified selection identities | `callsites_and_loop_iterations_have_distinct_selection_identities` |
| `counted_for` | Finite unrolling with iteration-qualified selection identities | `callsites_and_loop_iterations_have_distinct_selection_identities` |

## Excluded operations

These operations are outside the finite pure-value contract rather than missing
implementation work.

| Operation | Reason |
|---|---|
| `after_all` | Token and effect ordering is outside the pure-value contract |
| `assert` | Token-consuming diagnostic operation |
| `trace` | Token-consuming diagnostic operation |
| `cover` | Token-consuming diagnostic operation |
| `instantiation_input` | Block and instantiation semantics are outside scope |
| `instantiation_output` | Block and instantiation semantics are outside scope |
| `register_read` | Implicit persistent state is outside scope |
| `register_write` | Implicit persistent state is outside scope |
