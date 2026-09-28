//! The two BIF lists are one set: `cfparse`' membership list
//! (`data/support_functions.json`, lowercase: which calls are builtins) and
//! this crate's casing table (`data/functions.json`, cfdocs spelling: how a
//! builtin is written). A name in one and not the other would be a builtin
//! the printer cannot case, or a casing no call reaches.

use std::collections::BTreeSet;

fn names(path: &str) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

#[test]
fn the_casing_table_is_the_builtin_list() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let casing = names(&format!("{dir}/data/functions.json"));
    let builtins = names(&format!("{dir}/../cfparse/data/support_functions.json"));
    let lowered: BTreeSet<String> = casing.iter().map(|n| n.to_ascii_lowercase()).collect();
    assert_eq!(
        lowered.len(),
        casing.len(),
        "a name twice in functions.json"
    );
    let builtins: BTreeSet<String> = builtins.into_iter().collect();
    let only_casing: Vec<_> = lowered.difference(&builtins).collect();
    let only_builtins: Vec<_> = builtins.difference(&lowered).collect();
    assert!(
        only_casing.is_empty() && only_builtins.is_empty(),
        "only in functions.json: {only_casing:?}\nonly in support_functions.json: {only_builtins:?}"
    );
}
