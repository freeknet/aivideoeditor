// ============================================================================
// Module:       test_support
// Description:  Shared helpers for the unit tests: running a proptest
//               property as an `anyhow` result.
//
// Dependencies: anyhow, proptest (dev-dependencies)
// ============================================================================

//! Test helpers, compiled only for `cargo test`.
//!
//! Tests here return `anyhow::Result<()>` and never panic on purpose, so a
//! property runs through a `TestRunner` and its failure (with the shrunk
//! input) comes back as an error rather than through the `proptest!`
//! macro's panic.

use proptest::strategy::Strategy;
use proptest::test_runner::{TestCaseError, TestRunner};

/// Runs `test` over `strategy`'s values (256 cases, shrinking on failure),
/// as an `anyhow` result.
pub(crate) fn property<S: Strategy>(
    strategy: S,
    test: impl Fn(S::Value) -> Result<(), TestCaseError>,
) -> anyhow::Result<()>
where
    S::Value: std::fmt::Debug,
{
    // No regression files: a failure prints its shrunk input, and the case
    // that found a bug becomes an ordinary test beside the fix.
    let config = proptest::test_runner::Config {
        failure_persistence: None,
        ..proptest::test_runner::Config::default()
    };
    TestRunner::new(config)
        .run(&strategy, test)
        .map_err(|error| anyhow::anyhow!("{error}"))
}

/// Any JSON value, nested a few levels: what another program could send.
pub(crate) fn any_json() -> impl Strategy<Value = serde_json::Value> {
    use proptest::prelude::*;
    use serde_json::Value;
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        ".{0,12}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::btree_map(".{0,8}", inner, 0..4)
                .prop_map(|map| Value::Object(map.into_iter().collect())),
        ]
    })
}
