// SPDX-License-Identifier: Apache-2.0

//! Reproducible bounded-path performance smoke test for the v1 release.

use std::time::{Duration, Instant};

use xlsynth::IrPackage;
use xlsynth_symex::{EnumerationCompleteness, enumerate_package};

#[test]
fn sixty_four_path_one_hot_stays_within_the_release_ceiling() {
    let ir = r#"package release_metrics

top fn one_hot_64(
    selector: bits[6] id=1,
    a: bits[8] id=2,
    b: bits[8] id=3,
    c: bits[8] id=4,
    d: bits[8] id=5,
    e: bits[8] id=6,
    f: bits[8] id=7
) -> bits[8] {
  ret result: bits[8] = one_hot_sel(selector, cases=[a, b, c, d, e, f], id=8)
}
"#;
    let package = IrPackage::parse_ir(ir, None).unwrap();
    let started = Instant::now();
    let result = enumerate_package(&package, "one_hot_64").unwrap();
    let elapsed = started.elapsed();

    assert_eq!(result.completeness, EnumerationCompleteness::Complete);
    assert_eq!(result.paths.len(), 64);
    assert_eq!(result.statistics.symbolic_outcomes, 64);
    assert_eq!(result.statistics.solver_queries, 64);
    assert_eq!(result.statistics.infeasible_candidates, 0);
    assert!(
        elapsed <= Duration::from_secs(30),
        "64-path enumeration took {elapsed:?}, exceeding the v1 smoke-test ceiling"
    );

    eprintln!(
        "v1_path_stress paths={} elapsed_ms={} construction_ms={} solver_ms={} expression_nodes={} evaluated_nodes={} cache_hits={}",
        result.paths.len(),
        elapsed.as_millis(),
        result.statistics.construction_time.as_millis(),
        result.statistics.solver_time.as_millis(),
        result.statistics.expression_nodes,
        result.statistics.evaluated_nodes,
        result.statistics.cache_hits,
    );
}
