//! Function-name casing.
//!
//! `function_call.casing.builtin` rewrites the name of a built-in function
//! call to its cfdocs spelling (`data/functions.json`, CommandBox's list) or
//! to that spelling with an upper-case first letter; a name missing from the
//! list is left as written. `function_call.casing.userdefined` changes the
//! first letter of a user-defined function call (`camel`: lower, `pascal`:
//! upper), except for a name starting with two upper-case letters
//! (`DOSomething`). Only plain calls are touched; a method call (`a.b()`) is
//! a chain segment, never a `CallExpr` callee, so its name is never cased.

use std::collections::HashMap;
use std::sync::OnceLock;

use crate::options::{BuiltinCasing, UserDefinedCasing};

/// Lower-cased name → cfdocs spelling, built once.
fn builtins() -> &'static HashMap<String, String> {
    static MAP: OnceLock<HashMap<String, String>> = OnceLock::new();
    MAP.get_or_init(|| {
        let names: Vec<String> = serde_json::from_str(include_str!("../../data/functions.json"))
            .expect("data/functions.json is a list of names");
        names.into_iter().map(|n| (n.to_lowercase(), n)).collect()
    })
}

/// The name of a built-in function call in `casing`.
pub(crate) fn builtin(name: &str, casing: BuiltinCasing) -> String {
    let canonical = match casing {
        BuiltinCasing::Ignored => return name.to_owned(),
        _ => match builtins().get(&name.to_lowercase()) {
            Some(c) => c,
            None => return name.to_owned(),
        },
    };
    match casing {
        BuiltinCasing::Pascal => upper_first(canonical),
        _ => canonical.clone(),
    }
}

/// The name of a user-defined function call in `casing`.
pub(crate) fn user_defined(name: &str, casing: UserDefinedCasing) -> String {
    let mut chars = name.chars();
    let two_upper = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.next().is_some_and(|c| c.is_ascii_uppercase());
    match casing {
        _ if two_upper => name.to_owned(),
        UserDefinedCasing::Ignored => name.to_owned(),
        UserDefinedCasing::Camel => lower_first(name),
        UserDefinedCasing::Pascal => upper_first(name),
    }
}

fn upper_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

fn lower_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_lowercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_casing() {
        assert_eq!(builtins().len(), 798);
        assert_eq!(builtin("ARRAYAPPEND", BuiltinCasing::Cfdocs), "arrayAppend");
        assert_eq!(builtin("arrayappend", BuiltinCasing::Pascal), "ArrayAppend");
        assert_eq!(
            builtin("ARRAYAPPEND", BuiltinCasing::Ignored),
            "ARRAYAPPEND"
        );
        assert_eq!(builtin("notABuiltin", BuiltinCasing::Cfdocs), "notABuiltin");
    }

    #[test]
    fn user_defined_casing() {
        assert_eq!(
            user_defined("DoSomething", UserDefinedCasing::Camel),
            "doSomething"
        );
        assert_eq!(
            user_defined("doSomething", UserDefinedCasing::Pascal),
            "DoSomething"
        );
        assert_eq!(
            user_defined("DOSomething", UserDefinedCasing::Camel),
            "DOSomething"
        );
        assert_eq!(
            user_defined("DoSomething", UserDefinedCasing::Ignored),
            "DoSomething"
        );
        assert_eq!(user_defined("$$test", UserDefinedCasing::Pascal), "$$test");
    }
}
