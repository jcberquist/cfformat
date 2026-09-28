//! The tag attributes that name a variable the tag writes.

/// Per tag, keyed by its name without the `cf` prefix and in lower case so
/// one entry serves `<cfquery name="q">`, `query name="q" {}` and
/// `cfquery(name="q")`: the attributes whose value is the name of a variable
/// the tag creates or overwrites. A list to grow. `thread` is absent on
/// purpose: its `name` names the thread, and its body is not a function's.
pub(crate) const RESULT_ATTRIBUTES: &[(&str, &[&str])] = &[
    ("cache", &["name", "metadata"]),
    ("chart", &["name"]),
    ("collection", &["name"]),
    ("dbinfo", &["name"]),
    ("directory", &["name"]),
    ("document", &["name"]),
    ("execute", &["variable", "errorvariable"]),
    ("feed", &["name", "query", "properties"]),
    ("file", &["variable", "result"]),
    ("ftp", &["name", "result"]),
    ("http", &["result", "name"]),
    ("image", &["name", "structname"]),
    ("imap", &["name"]),
    ("index", &["status"]),
    ("invoke", &["returnvariable"]),
    ("ldap", &["name"]),
    // Lucee's struct loop names `key` and `value`.
    ("loop", &["index", "item", "key", "value"]),
    ("ntauthenticate", &["result"]),
    ("object", &["name"]),
    ("param", &["name"]),
    ("pdf", &["name"]),
    ("pop", &["name"]),
    ("procparam", &["variable"]),
    ("procresult", &["name"]),
    ("query", &["name", "result"]),
    ("registry", &["variable", "name"]),
    ("report", &["name"]),
    ("savecontent", &["variable"]),
    ("search", &["name"]),
    ("sharepoint", &["name"]),
    ("spreadsheet", &["name", "query"]),
    ("storedproc", &["result"]),
    ("wddx", &["output"]),
    ("xml", &["variable"]),
    ("zip", &["name", "variable"]),
];

/// The attributes of `tag` (any case, `cf` prefix or not) that name a
/// variable it writes; empty for a tag not in the table.
pub(crate) fn result_attributes(tag: &str) -> &'static [&'static str] {
    let bare = match tag.get(..2) {
        Some(cf) if cf.eq_ignore_ascii_case("cf") => &tag[2..],
        _ => tag,
    };
    RESULT_ATTRIBUTES
        .iter()
        .find(|(name, _)| bare.eq_ignore_ascii_case(name))
        .map_or(&[], |(_, attributes)| attributes)
}
