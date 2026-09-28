//! The project's formatter configuration for `"oxc"` (`islands.config`):
//! `.oxfmtrc.json` / `.oxfmtrc.jsonc`, or prettier's
//! `.prettierrc` family, found from the file's directory upward and read into
//! an [`IslandConfig`] that [`super::Oxc`] applies over its layout.
//!
//! The search is prettier's with oxfmt's two names first in each
//! directory: the first directory holding any candidate wins, whatever
//! that file's state. A file that
//! cannot be read, or is in a format this module does not read (YAML, JSON5,
//! TOML, JavaScript), is a warning once per run and oxc's defaults apply;
//! the walk never goes on to another project's file. JSON is read with
//! comments and trailing commas (oxfmt reads comments; prettier loads an
//! extensionless `.prettierrc` as YAML, which accepts the trailing comma).
//!
//! The keys read are those oxfmt maps onto the oxc formatter; `printWidth`,
//! `tabWidth` and `useTabs` are not read (cfformat's `max_columns`,
//! `indent_size` and `tab_indent` win) and neither are `endOfLine`,
//! `plugins` or any key oxc has no use for.
//! This file names no oxc type: the mapping onto oxc's option structs is in
//! `oxc.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde_json::{Map, Value};

use crate::options::IslandConfigMode;

/// `quoteProps`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum QuoteProps {
    AsNeeded,
    Consistent,
    Preserve,
}

/// `trailingComma`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TrailingComma {
    All,
    Es5,
    None,
}

/// `arrowParens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ArrowParens {
    Always,
    Avoid,
}

/// `objectWrap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum ObjectWrap {
    Preserve,
    Collapse,
}

/// `experimentalOperatorPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum OperatorPosition {
    Start,
    End,
}

/// The options of a project's configuration file that `"oxc"` applies to
/// an island, each `None` where the file does not set it. The default is the
/// empty configuration: oxc's defaults, which are prettier's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct IslandConfig {
    pub(crate) single_quote: Option<bool>,
    pub(crate) jsx_single_quote: Option<bool>,
    pub(crate) quote_props: Option<QuoteProps>,
    pub(crate) trailing_comma: Option<TrailingComma>,
    pub(crate) semi: Option<bool>,
    pub(crate) arrow_parens: Option<ArrowParens>,
    pub(crate) bracket_spacing: Option<bool>,
    pub(crate) bracket_same_line: Option<bool>,
    pub(crate) single_attribute_per_line: Option<bool>,
    pub(crate) object_wrap: Option<ObjectWrap>,
    pub(crate) operator_position: Option<OperatorPosition>,
}

impl IslandConfig {
    /// Every option `over` sets replaces this one's (prettier's
    /// `Object.assign` of an override's `options`).
    fn layer(&mut self, over: &IslandConfig) {
        macro_rules! take {
            ($($field:ident),*) => {$(
                if over.$field.is_some() {
                    self.$field = over.$field;
                }
            )*};
        }
        take!(
            single_quote,
            jsx_single_quote,
            quote_props,
            trailing_comma,
            semi,
            arrow_parens,
            bracket_spacing,
            bracket_same_line,
            single_attribute_per_line,
            object_wrap,
            operator_position
        );
    }

    /// Reads the options from a configuration object; a value of the wrong
    /// type or an unknown string is a warning (`` `key`: … ``) and the key
    /// is ignored. `at` prefixes the key in messages (`overrides[0].options.`).
    fn from_object(map: &Map<String, Value>, at: &str, warnings: &mut Vec<String>) -> Self {
        let mut config = IslandConfig::default();
        for (key, value) in map {
            let mut bad = |expected: &str| {
                warnings.push(format!(
                    "`{at}{key}`: expected {expected}, not {value}; ignored"
                ));
            };
            let flag = |bad: &mut dyn FnMut(&str)| match value {
                Value::Bool(b) => Some(*b),
                _ => {
                    bad("true or false");
                    None
                }
            };
            macro_rules! choice {
                ($($name:literal => $variant:expr),*) => {
                    match value.as_str() {
                        $(Some($name) => Some($variant),)*
                        _ => {
                            bad(&one_of(&[$($name),*]));
                            None
                        }
                    }
                };
            }
            match key.as_str() {
                "singleQuote" => config.single_quote = flag(&mut bad),
                "jsxSingleQuote" => config.jsx_single_quote = flag(&mut bad),
                "semi" => config.semi = flag(&mut bad),
                "bracketSpacing" => config.bracket_spacing = flag(&mut bad),
                "bracketSameLine" => config.bracket_same_line = flag(&mut bad),
                "singleAttributePerLine" => config.single_attribute_per_line = flag(&mut bad),
                "quoteProps" => {
                    config.quote_props = choice!(
                        "as-needed" => QuoteProps::AsNeeded,
                        "consistent" => QuoteProps::Consistent,
                        "preserve" => QuoteProps::Preserve
                    )
                }
                "trailingComma" => {
                    config.trailing_comma = choice!(
                        "all" => TrailingComma::All,
                        "es5" => TrailingComma::Es5,
                        "none" => TrailingComma::None
                    )
                }
                "arrowParens" => {
                    config.arrow_parens = choice!(
                        "always" => ArrowParens::Always,
                        "avoid" => ArrowParens::Avoid
                    )
                }
                "objectWrap" => {
                    config.object_wrap = choice!(
                        "preserve" => ObjectWrap::Preserve,
                        "collapse" => ObjectWrap::Collapse
                    )
                }
                "experimentalOperatorPosition" => {
                    config.operator_position = choice!(
                        "start" => OperatorPosition::Start,
                        "end" => OperatorPosition::End
                    )
                }
                // oxfmt rejects it; the oxc formatter does not implement it.
                "experimentalTernaries" if *value == Value::Bool(true) => warnings.push(format!(
                    "`{at}experimentalTernaries`: not supported by oxc; ignored"
                )),
                // `printWidth`, `tabWidth`, `useTabs` (cfformat's layout
                // wins), `endOfLine`, `plugins`, prettier's options for
                // other languages, plugin options: not read.
                _ => {}
            }
        }
        config
    }
}

/// `"a", "b" or "c"`.
fn one_of(names: &[&str]) -> String {
    let quoted: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
    match quoted.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// An override's `files` or `excludeFiles`, prettier's way: a pattern
/// without `/` matches the basename, one with `/` the path relative to the
/// configuration file's directory; `*` matches a leading dot. The syntax
/// read is `globset`'s: `*`, `**`, `?`, `[…]` and `{a,b}`; prettier's
/// micromatch extglobs (`@(…)`, `+(…)`, `!(…)`, `?(…)`, `*(…)`) are not,
/// and a pattern holding one is skipped with a warning.
#[derive(Debug)]
struct Patterns {
    basename: GlobSet,
    relative: GlobSet,
}

impl Patterns {
    /// An error for a value that is not a string or an array of strings, or
    /// holds an invalid glob (the message says which); the patterns
    /// skipped for their extglob syntax beside the set.
    fn new(value: &Value) -> Result<(Self, Vec<String>), String> {
        let list: Vec<&str> = match value {
            Value::String(s) => vec![s.as_str()],
            Value::Array(items) => items
                .iter()
                .map(|v| v.as_str().ok_or("expected a string or an array of strings"))
                .collect::<Result<_, _>>()?,
            _ => return Err("expected a string or an array of strings".into()),
        };
        let (mut basename, mut relative) = (GlobSetBuilder::new(), GlobSetBuilder::new());
        let mut skipped = Vec::new();
        for pattern in list {
            if ["@(", "+(", "!(", "?(", "*("]
                .iter()
                .any(|ext| pattern.contains(ext))
            {
                skipped.push(pattern.to_owned());
                continue;
            }
            let glob = GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
                .map_err(|e| e.to_string())?;
            if pattern.contains('/') {
                relative.add(glob);
            } else {
                basename.add(glob);
            }
        }
        let patterns = Patterns {
            basename: basename.build().map_err(|e| e.to_string())?,
            relative: relative.build().map_err(|e| e.to_string())?,
        };
        Ok((patterns, skipped))
    }

    fn is_match(&self, relative: &Path) -> bool {
        relative
            .file_name()
            .is_some_and(|name| self.basename.is_match(name))
            || self.relative.is_match(relative)
    }
}

/// One entry of `overrides`.
#[derive(Debug)]
struct Override {
    files: Patterns,
    exclude: Option<Patterns>,
    options: IslandConfig,
}

/// A configuration file as read: its options and overrides, empty when it
/// could not be read (its warnings say why).
#[derive(Debug, Default)]
struct Parsed {
    /// The file's directory, which `overrides` patterns are relative to.
    dir: PathBuf,
    base: Arc<IslandConfig>,
    overrides: Vec<Override>,
}

impl Parsed {
    /// The configuration for the island at `path` (the synthetic path):
    /// the top-level options with every matching override's layered over
    /// them, first to last.
    fn resolve(&self, path: &Path) -> Arc<IslandConfig> {
        let Ok(relative) = path.strip_prefix(&self.dir) else {
            return Arc::clone(&self.base);
        };
        let mut matched = self
            .overrides
            .iter()
            .filter(|o| {
                o.files.is_match(relative)
                    && !o.exclude.as_ref().is_some_and(|e| e.is_match(relative))
            })
            .peekable();
        if matched.peek().is_none() {
            return Arc::clone(&self.base);
        }
        let mut config = (*self.base).clone();
        for o in matched {
            config.layer(&o.options);
        }
        Arc::new(config)
    }
}

/// The candidates in one directory, in the order they are checked:
/// oxfmt's names, then prettier's `CONFIG_FILES`.
const CANDIDATES: &[&str] = &[
    ".oxfmtrc.json",
    ".oxfmtrc.jsonc",
    "package.json",
    "package.yaml",
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yml",
    ".prettierrc.yaml",
    ".prettierrc.json5",
    ".prettierrc.js",
    "prettier.config.js",
    ".prettierrc.ts",
    "prettier.config.ts",
    ".prettierrc.mjs",
    "prettier.config.mjs",
    ".prettierrc.mts",
    "prettier.config.mts",
    ".prettierrc.cjs",
    "prettier.config.cjs",
    ".prettierrc.cts",
    "prettier.config.cts",
    ".prettierrc.toml",
];

/// The configuration file that applies in `dir` itself, if any. A
/// `package.json` counts only with a `"prettier"` value that is not `null`
/// or `false`, and a `package.yaml` only with a top-level `prettier:` line
/// whose value is not `false` or `null` either ([`yaml_prettier_key`]);
/// prettier's filter. A `package.json` that is not JSON is not a hit.
/// When a `package.*` is not, the next name in the same directory is
/// checked.
fn candidate_in(dir: &Path) -> Option<PathBuf> {
    CANDIDATES.iter().map(|name| dir.join(name)).find(|path| {
        if !path.is_file() {
            return false;
        }
        match path.file_name().and_then(|n| n.to_str()) {
            Some("package.json") => std::fs::read_to_string(path)
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok())
                .and_then(|json| json.get("prettier").cloned())
                .is_some_and(|v| !matches!(v, Value::Null | Value::Bool(false))),
            Some("package.yaml") => {
                std::fs::read_to_string(path).is_ok_and(|text| yaml_prettier_key(&text))
            }
            _ => true,
        }
    })
}

/// Whether a `package.yaml` has a top-level `prettier:` key with a value:
/// anything after the colon but `false`, `null` or `~`, a `#` comment
/// removed; or nothing there and a block on the next lines — the next line
/// that is not blank or a `#` comment is indented (a mapping or sequence) or
/// a `-` item (a sequence at the key's indentation). A bare `prettier:`
/// followed by another top-level key or the end of the file is YAML null.
/// The file is still YAML, which is not read: a hit warns.
fn yaml_prettier_key(text: &str) -> bool {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(rest) = line.strip_prefix("prettier:") else {
            continue;
        };
        let value = without_comment(rest).trim();
        if !value.is_empty() {
            return !(value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("null")
                || value == "~");
        }
        let next = lines
            .clone()
            .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        return next.is_some_and(|l| {
            l.starts_with(char::is_whitespace) || l == "-" || l.starts_with("- ")
        });
    }
    false
}

/// A YAML line's text before its `#` comment (a `#` at the start or after
/// whitespace).
fn without_comment(rest: &str) -> &str {
    match rest
        .char_indices()
        .find(|&(i, c)| c == '#' && (i == 0 || rest[..i].ends_with(char::is_whitespace)))
    {
        Some((i, _)) => &rest[..i],
        None => rest,
    }
}

/// Reads `path` into its configuration; everything wrong with it is in
/// `warnings`.
fn parse(path: &Path, warnings: &mut Vec<String>) -> Parsed {
    let mut parsed = Parsed {
        dir: path.parent().map(PathBuf::from).unwrap_or_default(),
        ..Parsed::default()
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let unsupported = match ext {
        "yml" | "yaml" => Some("YAML"),
        "json5" => Some("JSON5"),
        "toml" => Some("TOML"),
        "js" | "ts" | "mjs" | "mts" | "cjs" | "cts" => Some("a JavaScript configuration"),
        _ => None,
    };
    if let Some(what) = unsupported {
        warnings.push(format!(
            "not read: {what} is not supported; oxc's defaults apply"
        ));
        return parsed;
    }
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            warnings.push(format!("cannot be read: {e}; oxc's defaults apply"));
            return parsed;
        }
    };
    let yaml = || "not JSON, and YAML is not read; oxc's defaults apply".to_string();
    let plain = match strip_jsonc(&text) {
        Ok(plain) => plain,
        // A YAML `.prettierrc` can hold an unquoted `/*` (`src/*/x`).
        Err(_) if name == ".prettierrc" => {
            warnings.push(yaml());
            return parsed;
        }
        Err(e) => {
            warnings.push(format!("not JSON: {e}; oxc's defaults apply"));
            return parsed;
        }
    };
    let json = match serde_json::from_str::<Value>(&plain) {
        Ok(json) => json,
        Err(_) if name == ".prettierrc" => {
            warnings.push(yaml());
            return parsed;
        }
        Err(e) => {
            warnings.push(format!("not JSON: {e}; oxc's defaults apply"));
            return parsed;
        }
    };
    let json = if name == "package.json" {
        match json.get("prettier") {
            Some(Value::String(_)) => {
                warnings.push(
                    "`prettier`: not read: a shared configuration package is not supported; \
                     oxc's defaults apply"
                        .into(),
                );
                return parsed;
            }
            Some(v) => v.clone(),
            None => return parsed,
        }
    } else {
        json
    };
    let Value::Object(mut map) = json else {
        warnings.push("not a JSON object; oxc's defaults apply".into());
        return parsed;
    };
    let overrides = map.remove("overrides");
    parsed.base = Arc::new(IslandConfig::from_object(&map, "", warnings));
    match overrides {
        None => {}
        Some(Value::Array(list)) => {
            for (i, entry) in list.iter().enumerate() {
                let at = format!("overrides[{i}]");
                match read_override(entry, &at, warnings) {
                    Ok(o) => parsed.overrides.push(o),
                    Err(message) => warnings.push(format!("`{at}`: {message}; ignored")),
                }
            }
        }
        Some(_) => warnings.push("`overrides`: expected an array; ignored".into()),
    }
    parsed
}

/// One `overrides` entry: `{ files, excludeFiles?, options }`.
fn read_override(entry: &Value, at: &str, warnings: &mut Vec<String>) -> Result<Override, String> {
    let Value::Object(entry) = entry else {
        return Err("expected an object".into());
    };
    let mut skipped = Vec::new();
    let (files, s) = entry
        .get("files")
        .ok_or_else(|| "no `files`".to_string())
        .and_then(|v| Patterns::new(v).map_err(|e| format!("`files`: {e}")))?;
    skipped.extend(s.into_iter().map(|p| ("files", p)));
    let exclude = match entry.get("excludeFiles") {
        None => None,
        Some(v) => {
            let (exclude, s) = Patterns::new(v).map_err(|e| format!("`excludeFiles`: {e}"))?;
            skipped.extend(s.into_iter().map(|p| ("excludeFiles", p)));
            Some(exclude)
        }
    };
    let options = match entry.get("options") {
        None => IslandConfig::default(),
        Some(Value::Object(map)) => {
            IslandConfig::from_object(map, &format!("{at}.options."), warnings)
        }
        Some(_) => return Err("`options`: expected an object".into()),
    };
    for (key, pattern) in skipped {
        let message = format!(
            "`{at}`: `{key}`: unsupported glob syntax in {pattern:?}; the pattern matches nothing"
        );
        // A pattern repeated in one list warns once.
        if !warnings.contains(&message) {
            warnings.push(message);
        }
    }
    Ok(Override {
        files,
        exclude,
        options,
    })
}

/// JSON with `//` and `/* */` comments and trailing commas made JSON: the
/// comments become spaces (their newlines kept, so serde's positions still
/// hold) and a comma before `]` or `}` goes. Strings are left as they are.
/// A `/*` that is never closed is an error.
fn strip_jsonc(text: &str) -> Result<String, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // Comments out.
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                plain.push(c);
                while let Some(c) = chars.next() {
                    plain.push(c);
                    match c {
                        '\\' => plain.extend(chars.next()),
                        '"' => break,
                        _ => {}
                    }
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        plain.push('\n');
                        break;
                    }
                    plain.push(if c == '\r' { '\r' } else { ' ' });
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                plain.push_str("  ");
                let mut last = ' ';
                let mut closed = false;
                for c in chars.by_ref() {
                    plain.push(if c == '\n' || c == '\r' { c } else { ' ' });
                    if last == '*' && c == '/' {
                        closed = true;
                        break;
                    }
                    last = c;
                }
                if !closed {
                    return Err("unterminated block comment".into());
                }
            }
            _ => plain.push(c),
        }
    }
    // Trailing commas out.
    let bytes = plain.as_bytes();
    let mut out = String::with_capacity(plain.len());
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in plain.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
        } else if c == '"' {
            in_string = true;
        } else if c == ',' {
            let next = bytes[i + 1..]
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .copied();
            if matches!(next, Some(b']' | b'}')) {
                out.push(' ');
                continue;
            }
        }
        out.push(c);
    }
    Ok(out)
}

/// Configuration discovery for one run, on the [`super::Islands`]: which
/// file applies to a directory and what each file holds, each looked up
/// once; the warnings of every file read, once per file.
#[derive(Debug, Default)]
pub(crate) struct ConfigCache {
    /// Directory → the configuration file that applies there (`None`: the
    /// walk to the root found none).
    dirs: HashMap<PathBuf, Option<PathBuf>>,
    /// Configuration file → its contents.
    files: HashMap<PathBuf, Arc<Parsed>>,
}

/// The empty configuration, shared.
fn empty() -> Arc<IslandConfig> {
    static EMPTY: std::sync::OnceLock<Arc<IslandConfig>> = std::sync::OnceLock::new();
    Arc::clone(EMPTY.get_or_init(Arc::default))
}

impl ConfigCache {
    /// The nearest directory's configuration file, from `dir` upward.
    fn nearest(&mut self, dir: &Path) -> Option<PathBuf> {
        let mut checked = Vec::new();
        let mut found = None;
        let mut next = Some(dir);
        while let Some(d) = next {
            if let Some(hit) = self.dirs.get(d) {
                found = hit.clone();
                break;
            }
            checked.push(d.to_path_buf());
            if let Some(file) = candidate_in(d) {
                found = Some(file);
                break;
            }
            next = d.parent();
        }
        for d in checked {
            self.dirs.insert(d, found.clone());
        }
        found
    }
}

/// The configuration for the island at `path` (absolute; its directory is
/// where the walk starts), reading and caching what the walk finds. New
/// warnings go to `warnings` as `(file, message)`.
pub(crate) fn resolve(
    cache: &Mutex<ConfigCache>,
    warnings: &Mutex<Vec<(PathBuf, String)>>,
    path: &Path,
    mode: IslandConfigMode,
) -> Arc<IslandConfig> {
    if mode == IslandConfigMode::Off {
        return empty();
    }
    let Some(dir) = path.parent() else {
        return empty();
    };
    let parsed = {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(file) = cache.nearest(dir) else {
            return empty();
        };
        match cache.files.get(&file) {
            Some(parsed) => Arc::clone(parsed),
            None => {
                let mut found = Vec::new();
                let parsed = Arc::new(parse(&file, &mut found));
                warnings
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(found.into_iter().map(|m| (file.clone(), m)));
                cache.files.insert(file, Arc::clone(&parsed));
                parsed
            }
        }
    };
    parsed.resolve(path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A fresh directory under the target's temp space.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("cfformat-config-{}", std::process::id()))
            .join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    struct Run {
        cache: Mutex<ConfigCache>,
        warnings: Mutex<Vec<(PathBuf, String)>>,
    }

    impl Run {
        fn new() -> Self {
            Run {
                cache: Mutex::default(),
                warnings: Mutex::default(),
            }
        }

        fn at(&self, path: &Path) -> IslandConfig {
            (*resolve(&self.cache, &self.warnings, path, IslandConfigMode::Auto)).clone()
        }

        fn warnings(&self) -> Vec<(PathBuf, String)> {
            self.warnings.lock().unwrap().clone()
        }
    }

    fn quote(single: bool) -> IslandConfig {
        IslandConfig {
            single_quote: Some(single),
            ..IslandConfig::default()
        }
    }

    #[test]
    fn strips_comments_and_trailing_commas() {
        let text = "\u{feff}{\r\n  // a comment with a \" quote\r\n  \"a\": \"x // y\", /* b */\n  \
                    \"c\": [1, 2,],\n  \"d\": \",]\\\" ,}\",\n  /* multi\n line */ \"e\": {\"f\": 1,},\n}\n";
        let json: Value = serde_json::from_str(&strip_jsonc(text).unwrap()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"a": "x // y", "c": [1, 2], "d": ",]\" ,}", "e": {"f": 1}})
        );
        // Positions hold: the lines are the same.
        assert_eq!(
            strip_jsonc(text).unwrap().lines().count(),
            text.lines().count()
        );
        // A `/*` never closed is an error, even when what is left is JSON.
        assert_eq!(
            strip_jsonc("{\"semi\": false} /* unfinished"),
            Err("unterminated block comment".into())
        );
        assert_eq!(
            strip_jsonc("{\"a\": \"/* in a string\"}").as_deref(),
            Ok("{\"a\": \"/* in a string\"}")
        );
    }

    #[test]
    fn an_unterminated_comment_is_not_json() {
        let root = scratch("unterminated");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        let dir = root.join("sub");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(".oxfmtrc.jsonc"),
            "{\"semi\": false} /* unfinished",
        )
        .unwrap();
        let run = Run::new();
        // The file applies — as nothing: the walk stops at it.
        assert_eq!(run.at(&dir.join("p.cfm.js")), IslandConfig::default());
        assert_eq!(
            run.warnings(),
            [(
                dir.join(".oxfmtrc.jsonc"),
                "not JSON: unterminated block comment; oxc's defaults apply".to_string()
            )]
        );
    }

    #[test]
    fn the_nearest_directory_wins() {
        let root = scratch("nearest");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/.prettierrc.json"), r#"{"singleQuote": false}"#).unwrap();
        let run = Run::new();
        assert_eq!(run.at(&root.join("a/b/page.cfm.js")), quote(false));
        assert_eq!(run.at(&root.join("page.cfm.js")), quote(true));
        // The walk from `b` answered `a` and `b`.
        assert_eq!(run.cache.lock().unwrap().files.len(), 2);
        assert!(run.warnings().is_empty());
    }

    #[test]
    fn the_order_within_a_directory() {
        let root = scratch("order");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": false}"#).unwrap();
        // A package.json without the key is not a hit.
        fs::write(root.join("package.json"), r#"{"name": "x"}"#).unwrap();
        assert_eq!(Run::new().at(&root.join("p.cfm.js")), quote(false));
        // With it, it comes before `.prettierrc`.
        fs::write(
            root.join("package.json"),
            r#"{"name": "x", "prettier": {"semi": false}}"#,
        )
        .unwrap();
        let semi = IslandConfig {
            semi: Some(false),
            ..IslandConfig::default()
        };
        assert_eq!(Run::new().at(&root.join("p.cfm.js")), semi);
        // And oxfmt's file before both.
        fs::write(root.join(".oxfmtrc.jsonc"), "// x\n{\"singleQuote\": true}").unwrap();
        assert_eq!(Run::new().at(&root.join("p.cfm.js")), quote(true));
        fs::write(root.join(".oxfmtrc.json"), r#"{"singleQuote": false}"#).unwrap();
        assert_eq!(Run::new().at(&root.join("p.cfm.js")), quote(false));
    }

    #[test]
    fn a_package_yaml_counts_only_with_a_prettier_value() {
        let root = scratch("package-yaml");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        // `prettier: false` (or null) does not mask the sibling `.prettierrc`.
        for value in ["false", "null", "~", "False  # off", "false#x"] {
            fs::write(
                root.join("package.yaml"),
                format!("name: x\nprettier: {value}\n"),
            )
            .unwrap();
            let run = Run::new();
            let expected = if value == "false#x" {
                // `#` without a space before it is not a comment.
                IslandConfig::default()
            } else {
                quote(true)
            };
            assert_eq!(run.at(&root.join("p.cfm.js")), expected, "{value}");
        }
        // A mapping on the next lines, or a value, is a hit: unread YAML.
        for text in [
            "prettier:\n  semi: false\n",
            "prettier: # below\n  semi: false\n",
            "prettier: \"@acme/prettier\"\n",
        ] {
            fs::write(root.join("package.yaml"), text).unwrap();
            let run = Run::new();
            assert_eq!(run.at(&root.join("p.cfm.js")), IslandConfig::default());
            assert_eq!(
                run.warnings(),
                [(
                    root.join("package.yaml"),
                    "not read: YAML is not supported; oxc's defaults apply".to_string()
                )],
                "{text:?}"
            );
        }
        // A bare `prettier:` before another top-level key or the end of the
        // file is YAML null: the sibling `.prettierrc` applies, no warning.
        for text in [
            "name: x\nprettier:\nversion: 1\n",
            "name: x\nprettier:\n",
            "prettier:",
            "prettier: # none\n\n# c\nversion: 1\n",
        ] {
            fs::write(root.join("package.yaml"), text).unwrap();
            let run = Run::new();
            assert_eq!(run.at(&root.join("p.cfm.js")), quote(true), "{text:?}");
            assert!(run.warnings().is_empty(), "{text:?}");
        }
        // A block after it, past blank lines and comments, is a value.
        for text in [
            "prettier:\n\n  # c\n  semi: false\n",
            "prettier:\n- a\n",
            "prettier:\n\tsemi: false\n",
        ] {
            fs::write(root.join("package.yaml"), text).unwrap();
            let run = Run::new();
            assert_eq!(
                run.at(&root.join("p.cfm.js")),
                IslandConfig::default(),
                "{text:?}"
            );
            assert_eq!(run.warnings().len(), 1, "{text:?}");
        }
        // An indented `prettier:` is not top-level.
        fs::write(root.join("package.yaml"), "config:\n  prettier: {}\n").unwrap();
        assert_eq!(Run::new().at(&root.join("p.cfm.js")), quote(true));
    }

    #[test]
    fn a_package_json_string_is_a_shared_configuration() {
        let root = scratch("shared");
        fs::write(
            root.join("package.json"),
            r#"{"prettier": "@acme/prettier"}"#,
        )
        .unwrap();
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        let run = Run::new();
        assert_eq!(run.at(&root.join("p.cfm.js")), IslandConfig::default());
        assert_eq!(
            run.warnings(),
            [(
                root.join("package.json"),
                "`prettier`: not read: a shared configuration package is not supported; \
                 oxc's defaults apply"
                    .to_string()
            )]
        );
    }

    #[test]
    fn unsupported_formats_warn_once_and_stop_the_walk() {
        let root = scratch("formats");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        for (name, what) in [
            (".prettierrc.yml", "YAML"),
            (".prettierrc.yaml", "YAML"),
            (".prettierrc.json5", "JSON5"),
            (".prettierrc.toml", "TOML"),
            ("prettier.config.js", "a JavaScript configuration"),
            (".prettierrc.cjs", "a JavaScript configuration"),
        ] {
            let dir = root.join(name.replace('.', "_"));
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join(name), "x").unwrap();
            let run = Run::new();
            assert_eq!(
                run.at(&dir.join("p.cfm.js")),
                IslandConfig::default(),
                "{name}"
            );
            assert_eq!(
                run.at(&dir.join("q.cfm.css")),
                IslandConfig::default(),
                "{name}"
            );
            assert_eq!(
                run.warnings(),
                [(
                    dir.join(name),
                    format!("not read: {what} is not supported; oxc's defaults apply")
                )]
            );
        }
        // An extensionless `.prettierrc` that is YAML.
        let dir = root.join("yaml");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".prettierrc"), "singleQuote: true\n").unwrap();
        let run = Run::new();
        assert_eq!(run.at(&dir.join("p.cfm.js")), IslandConfig::default());
        assert_eq!(
            run.warnings()[0].1,
            "not JSON, and YAML is not read; oxc's defaults apply"
        );
        // Even when its YAML holds what reads as an open `/*`.
        fs::write(
            dir.join(".prettierrc"),
            "overrides:\n  - files: src/*/x.js\n",
        )
        .unwrap();
        let run = Run::new();
        assert_eq!(run.at(&dir.join("p.cfm.js")), IslandConfig::default());
        assert_eq!(
            run.warnings()[0].1,
            "not JSON, and YAML is not read; oxc's defaults apply"
        );
        // `.prettierrc.json` is JSON or nothing.
        fs::write(dir.join(".prettierrc"), "").unwrap();
        fs::remove_file(dir.join(".prettierrc")).unwrap();
        fs::write(dir.join(".prettierrc.json"), "{\"a\": }").unwrap();
        let run = Run::new();
        assert_eq!(run.at(&dir.join("p.cfm.js")), IslandConfig::default());
        assert!(run.warnings()[0].1.starts_with("not JSON: "));
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_stops_the_walk() {
        use std::os::unix::fs::PermissionsExt;
        let root = scratch("unreadable");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        let dir = root.join("sub");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join(".prettierrc.json");
        fs::write(&file, "{}").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&file).is_ok() {
            // Running as root: permissions do not apply.
            return;
        }
        let run = Run::new();
        assert_eq!(run.at(&dir.join("p.cfm.js")), IslandConfig::default());
        let warnings = run.warnings();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].1.starts_with("cannot be read: "),
            "{warnings:?}"
        );
        fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
    }

    #[test]
    fn the_keys_read_and_the_bad_values() {
        let root = scratch("keys");
        fs::write(
            root.join(".prettierrc"),
            r#"{
                "singleQuote": true, "jsxSingleQuote": true, "quoteProps": "consistent",
                "trailingComma": "es5", "semi": false, "arrowParens": "avoid",
                "bracketSpacing": false, "bracketSameLine": true,
                "singleAttributePerLine": true, "objectWrap": "collapse",
                "experimentalOperatorPosition": "start",
                "printWidth": 40, "tabWidth": 8, "useTabs": true, "endOfLine": "crlf",
                "plugins": ["x"], "tailwindFunctions": ["clsx"],
            }"#,
        )
        .unwrap();
        let run = Run::new();
        assert_eq!(
            run.at(&root.join("p.cfm.js")),
            IslandConfig {
                single_quote: Some(true),
                jsx_single_quote: Some(true),
                quote_props: Some(QuoteProps::Consistent),
                trailing_comma: Some(TrailingComma::Es5),
                semi: Some(false),
                arrow_parens: Some(ArrowParens::Avoid),
                bracket_spacing: Some(false),
                bracket_same_line: Some(true),
                single_attribute_per_line: Some(true),
                object_wrap: Some(ObjectWrap::Collapse),
                operator_position: Some(OperatorPosition::Start),
            }
        );
        assert!(run.warnings().is_empty());

        fs::write(
            root.join(".prettierrc"),
            r#"{"semi": "no", "trailingComma": "some", "singleQuote": true,
                "experimentalTernaries": true}"#,
        )
        .unwrap();
        let run = Run::new();
        assert_eq!(run.at(&root.join("p.cfm.js")), quote(true));
        let messages: Vec<String> = run.warnings().into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            messages,
            [
                "`semi`: expected true or false, not \"no\"; ignored",
                "`trailingComma`: expected \"all\", \"es5\" or \"none\", not \"some\"; ignored",
                "`experimentalTernaries`: not supported by oxc; ignored",
            ]
        );
    }

    #[test]
    fn overrides_by_basename_and_by_path() {
        let root = scratch("overrides");
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(
            root.join(".prettierrc"),
            r#"{
                "singleQuote": true,
                "overrides": [
                    {"files": "*.css", "options": {"singleQuote": false}},
                    {"files": ["sub/**/*.js"], "excludeFiles": "skip.*", "options": {"semi": false}},
                    {"files": ".hidden.*", "options": {"semi": true}},
                    {"files": 3, "options": {}},
                    {"options": {}}
                ]
            }"#,
        )
        .unwrap();
        let run = Run::new();
        assert_eq!(run.at(&root.join("page.cfm.js")), quote(true));
        // A pattern without `/` matches the basename at any depth.
        assert_eq!(run.at(&root.join("sub/deep/page.cfm.css")), quote(false));
        let no_semi = IslandConfig {
            semi: Some(false),
            ..quote(true)
        };
        assert_eq!(run.at(&root.join("sub/deep/page.cfm.js")), no_semi);
        assert_eq!(run.at(&root.join("sub/page.cfm.js")), no_semi);
        assert_eq!(run.at(&root.join("sub/skip.cfm.js")), quote(true));
        // Dot files match.
        let semi = IslandConfig {
            semi: Some(true),
            ..quote(true)
        };
        assert_eq!(run.at(&root.join(".hidden.cfm.js")), semi);
        let messages: Vec<String> = run.warnings().into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            messages,
            [
                "`overrides[3]`: `files`: expected a string or an array of strings; ignored",
                "`overrides[4]`: no `files`; ignored",
            ]
        );
    }

    #[test]
    fn an_extglob_warns_and_matches_nothing() {
        let root = scratch("extglob");
        fs::write(
            root.join(".prettierrc"),
            r#"{
                "overrides": [
                    {"files": ["*.@(js|mjs)", "*.css"], "options": {"semi": false}},
                    {"files": "*.js", "excludeFiles": "!(page).*", "options": {"singleQuote": true}}
                ]
            }"#,
        )
        .unwrap();
        let run = Run::new();
        // The extglob is skipped; the rest of the entry still applies.
        assert_eq!(run.at(&root.join("page.cfm.js")), quote(true));
        let no_semi = IslandConfig {
            semi: Some(false),
            ..IslandConfig::default()
        };
        assert_eq!(run.at(&root.join("page.cfm.css")), no_semi);
        let messages: Vec<String> = run.warnings().into_iter().map(|(_, m)| m).collect();
        assert_eq!(
            messages,
            [
                "`overrides[0]`: `files`: unsupported glob syntax in \"*.@(js|mjs)\"; \
                 the pattern matches nothing",
                "`overrides[1]`: `excludeFiles`: unsupported glob syntax in \"!(page).*\"; \
                 the pattern matches nothing",
            ]
        );
    }

    #[test]
    fn off_and_nothing_found_are_empty() {
        let root = scratch("off");
        fs::write(root.join(".prettierrc"), r#"{"singleQuote": true}"#).unwrap();
        let run = Run::new();
        let off = resolve(
            &run.cache,
            &run.warnings,
            &root.join("p.cfm.js"),
            IslandConfigMode::Off,
        );
        assert_eq!(*off, IslandConfig::default());
        assert!(
            run.cache.lock().unwrap().dirs.is_empty(),
            "off walks nowhere"
        );
        // The temp directory's ancestors hold no configuration.
        let bare = scratch("bare");
        assert_eq!(run.at(&bare.join("p.cfm.js")), IslandConfig::default());
    }
}
