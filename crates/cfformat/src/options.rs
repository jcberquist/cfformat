//! Formatter options: the flat dotted-key JSON object of `.cfformat.json`,
//! CommandBox cfformat's keys with some removed (their behaviour fixed) and
//! some merged or renamed. The defaults are cfformat's own (`SETTINGS.md`,
//! at the repository root); several differ from CommandBox's.
//!
//! Loading is two steps. [`Options::migrate`] rewrites the raw JSON object:
//! removed keys are dropped with a [`Warning`] spelling out the
//! fixed behaviour, renamed keys move to their new names, and old values
//! (the booleans of `strings.convert_nested_quotes`) become the new ones.
//! The migrated object is then deserialised into [`Options`], where an
//! unknown key is an error, and validated. [`Options::from_json`] does both.
//!
//! [`Discovery`] resolves the options of a file: the nearest `.cfformat.json`
//! walking up from the file (stopping at a `.git` directory), or
//! `~/.cfformat.json` when the walk finds none, then an explicit `--config`
//! file over it, merged key by key.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod reference;

pub use reference::{reference, Example, ExampleMode, Kind, OptionInfo, SEPARATOR_EXAMPLES};

/// Every formatter option. Field names follow the JSON keys with `.`
/// replaced by `_`. The keys come from CommandBox cfformat's `.cfformat.json`;
/// the defaults are cfformat's own, listed in `SETTINGS.md`, and differ from
/// CommandBox's where the output was meant to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Options {
    /// Columns per indent level (`indent_size`).
    #[serde(rename = "indent_size")]
    pub indent_size: usize,
    /// Indent with tabs instead of spaces (`tab_indent`).
    #[serde(rename = "tab_indent")]
    pub tab_indent: bool,
    /// Line width the printer tries to stay within (`max_columns`).
    #[serde(rename = "max_columns")]
    pub max_columns: usize,
    /// Line ending of the output (`newline`).
    #[serde(rename = "newline")]
    pub newline: NewlineStyle,

    /// Quote style of strings in script (`strings.quote`).
    #[serde(rename = "strings.quote")]
    pub strings_quote: QuoteStyle,
    /// Quote style of tag and script-tag attribute values
    /// (`strings.attributes.quote`).
    #[serde(rename = "strings.attributes.quote")]
    pub strings_attributes_quote: QuoteStyle,
    /// The quote of a string that contains a quote character
    /// (`strings.convert_nested_quotes`, formerly `strings.convertNestedQuotes`).
    #[serde(rename = "strings.convert_nested_quotes")]
    pub strings_convert_nested_quotes: NestedQuotes,

    /// Key/value separator of struct members (`struct.separator`).
    #[serde(rename = "struct.separator")]
    pub struct_separator: String,
    /// Quote struct keys (`struct.quote_keys`).
    #[serde(rename = "struct.quote_keys")]
    pub struct_quote_keys: bool,
    /// Space inside non-empty struct braces (`struct.padding`).
    #[serde(rename = "struct.padding")]
    pub struct_padding: bool,
    /// Space inside empty struct braces, `{ }`, and an empty ordered
    /// struct, `[ : ]` (`struct.empty_padding`).
    #[serde(rename = "struct.empty_padding")]
    pub struct_empty_padding: bool,
    /// Break a struct with at least this many members, 0 never
    /// (`struct.multiline.element_count`)…
    #[serde(rename = "struct.multiline.element_count")]
    pub struct_multiline_element_count: u32,
    /// …whose items average more than this many columns one-line
    /// (`struct.multiline.min_item_length`).
    #[serde(rename = "struct.multiline.min_item_length")]
    pub struct_multiline_min_item_length: u32,

    /// Space inside non-empty array brackets (`array.padding`).
    #[serde(rename = "array.padding")]
    pub array_padding: bool,
    /// Space inside empty array brackets, `[ ]` (`array.empty_padding`).
    #[serde(rename = "array.empty_padding")]
    pub array_empty_padding: bool,
    /// `array.multiline.element_count`.
    #[serde(rename = "array.multiline.element_count")]
    pub array_multiline_element_count: u32,
    /// `array.multiline.min_item_length`.
    #[serde(rename = "array.multiline.min_item_length")]
    pub array_multiline_min_item_length: u32,

    /// Space inside non-empty index brackets, `a[ 1 ]` (`brackets.padding`).
    #[serde(rename = "brackets.padding")]
    pub brackets_padding: bool,

    /// Space inside non-empty parentheses: calls, parameters, groups and
    /// keyword groups (`parentheses.padding`; absorbs `function_call.padding`,
    /// `function_declaration.padding`, `function_anonymous.padding`).
    #[serde(rename = "parentheses.padding")]
    pub parentheses_padding: bool,

    /// Casing of built-in function calls (`function_call.casing.builtin`).
    #[serde(rename = "function_call.casing.builtin")]
    pub function_call_casing_builtin: BuiltinCasing,
    /// Casing of user-defined function calls
    /// (`function_call.casing.userdefined`).
    #[serde(rename = "function_call.casing.userdefined")]
    pub function_call_casing_userdefined: UserDefinedCasing,
    /// `function_call.multiline.element_count`.
    #[serde(rename = "function_call.multiline.element_count")]
    pub function_call_multiline_element_count: u32,
    /// `function_call.multiline.min_item_length`.
    #[serde(rename = "function_call.multiline.min_item_length")]
    pub function_call_multiline_min_item_length: u32,

    /// `function_declaration.multiline.element_count`.
    #[serde(rename = "function_declaration.multiline.element_count")]
    pub function_declaration_multiline_element_count: u32,
    /// `function_declaration.multiline.min_item_length`.
    #[serde(rename = "function_declaration.multiline.min_item_length")]
    pub function_declaration_multiline_min_item_length: u32,

    /// `function_anonymous.multiline.element_count`.
    #[serde(rename = "function_anonymous.multiline.element_count")]
    pub function_anonymous_multiline_element_count: u32,
    /// `function_anonymous.multiline.min_item_length`.
    #[serde(rename = "function_anonymous.multiline.min_item_length")]
    pub function_anonymous_multiline_min_item_length: u32,

    /// Component/function metadata attributes: `metadata.multiline.element_count`.
    #[serde(rename = "metadata.multiline.element_count")]
    pub metadata_multiline_element_count: u32,
    /// `metadata.multiline.min_item_length`.
    #[serde(rename = "metadata.multiline.min_item_length")]
    pub metadata_multiline_min_item_length: u32,
    /// `param` attributes: `param.multiline.element_count`.
    #[serde(rename = "param.multiline.element_count")]
    pub param_multiline_element_count: u32,
    /// `param.multiline.min_item_length`.
    #[serde(rename = "param.multiline.min_item_length")]
    pub param_multiline_min_item_length: u32,
    /// `property` attributes: `property.multiline.element_count`.
    #[serde(rename = "property.multiline.element_count")]
    pub property_multiline_element_count: u32,
    /// `property.multiline.min_item_length`.
    #[serde(rename = "property.multiline.min_item_length")]
    pub property_multiline_min_item_length: u32,
    /// Spaces around `=` in metadata, `param` and `property` attributes
    /// (`attributes.key_value.padding`; absorbs the three
    /// `*.key_value.padding` keys).
    #[serde(rename = "attributes.key_value.padding")]
    pub attributes_key_value_padding: bool,

    /// Lowercase CFML tag names (`tags.lowercase`).
    #[serde(rename = "tags.lowercase")]
    pub tags_lowercase: bool,
    /// Whether a paired CF tag whose body starts with HTML indents that body
    /// (`tags.body.indent`).
    #[serde(rename = "tags.body.indent")]
    pub tags_body_indent: TagBodyIndent,
    /// Whether the body of a paired `<cfscript>`, `<script>`, `<style>`,
    /// `<cfquery>` or `<cfjava>` tag is indented one level inside the tag
    /// rather than printed at the tag's own indent (`tags.islands.indent`).
    #[serde(rename = "tags.islands.indent")]
    pub tags_islands_indent: bool,
    /// Asterisk alignment in block and doc comments (`comment.asterisks`).
    #[serde(rename = "comment.asterisks")]
    pub comment_asterisks: Asterisks,
    /// Where the commas of a broken delimited list go (`multiline.comma`;
    /// replaces the per-construct `*.multiline.comma_dangle` and
    /// `*.multiline.leading_comma` keys).
    #[serde(rename = "multiline.comma")]
    pub multiline_comma: CommaStyle,
    /// Break a member chain with at least this many method calls; 0 leaves
    /// chains to the layout rules (`method_call.chain.multiline`).
    #[serde(rename = "method_call.chain.multiline")]
    pub method_call_chain_multiline: u32,
    /// Align `=` of consecutive assignments
    /// (`alignment.consecutive.assignments`).
    #[serde(rename = "alignment.consecutive.assignments")]
    pub alignment_consecutive_assignments: bool,
    /// Align the attributes of consecutive `param name=… type=…;` statements
    /// (`alignment.consecutive.params`).
    #[serde(rename = "alignment.consecutive.params")]
    pub alignment_consecutive_params: bool,
    /// Align the attributes of consecutive `property name=… inject=…;`
    /// statements (`alignment.consecutive.properties`).
    #[serde(rename = "alignment.consecutive.properties")]
    pub alignment_consecutive_properties: bool,
    /// Align `@param` descriptions in doc comments (`alignment.doc_comments`).
    #[serde(rename = "alignment.doc_comments")]
    pub alignment_doc_comments: bool,

    /// `<script>` islands (`islands.js`: `"oxc"`, the default, or `"off"`):
    /// pure ones, and under `islands.interpolated` those holding only `#…#`
    /// and `##`.
    #[serde(rename = "islands.js")]
    pub islands_js: IslandPreset,
    /// `<style>` islands (`islands.css`): pure ones, and under
    /// `islands.interpolated` those holding only `#…#` and `##`.
    #[serde(rename = "islands.css")]
    pub islands_css: IslandPreset,
    /// Pure JSON `<script>` islands (`islands.json`); one holding `#…#` or `##`
    /// is never handed off.
    #[serde(rename = "islands.json")]
    pub islands_json: IslandPreset,
    /// Whether a `<script>` / `<style>` island holding only text, `#…#` and
    /// `##` is handed to the island formatter (`islands.interpolated`); when
    /// false it prints as written, with no warning, and pure islands are
    /// still formatted.
    #[serde(rename = "islands.interpolated")]
    pub islands_interpolated: bool,
    /// Whether `"oxc"` reads the project's `.oxfmtrc` / `.prettierrc`
    /// (`islands.config`).
    #[serde(rename = "islands.config")]
    pub islands_config: IslandConfigMode,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            indent_size: 4,
            tab_indent: false,
            max_columns: 120,
            newline: NewlineStyle::Os,
            strings_quote: QuoteStyle::Single,
            strings_attributes_quote: QuoteStyle::Double,
            strings_convert_nested_quotes: NestedQuotes::FewerEscapes,
            struct_separator: ": ".into(),
            struct_quote_keys: false,
            struct_padding: false,
            struct_empty_padding: false,
            struct_multiline_element_count: 0,
            struct_multiline_min_item_length: 8,
            array_padding: false,
            array_empty_padding: false,
            array_multiline_element_count: 0,
            array_multiline_min_item_length: 8,
            brackets_padding: false,
            parentheses_padding: false,
            function_call_casing_builtin: BuiltinCasing::Cfdocs,
            function_call_casing_userdefined: UserDefinedCasing::Ignored,
            function_call_multiline_element_count: 0,
            function_call_multiline_min_item_length: 8,
            function_declaration_multiline_element_count: 0,
            function_declaration_multiline_min_item_length: 8,
            function_anonymous_multiline_element_count: 0,
            function_anonymous_multiline_min_item_length: 8,
            metadata_multiline_element_count: 0,
            metadata_multiline_min_item_length: 12,
            param_multiline_element_count: 0,
            param_multiline_min_item_length: 12,
            property_multiline_element_count: 0,
            property_multiline_min_item_length: 12,
            attributes_key_value_padding: false,
            tags_lowercase: true,
            tags_body_indent: TagBodyIndent::Always,
            tags_islands_indent: true,
            comment_asterisks: Asterisks::Align,
            multiline_comma: CommaStyle::Trailing,
            method_call_chain_multiline: 0,
            alignment_consecutive_assignments: false,
            alignment_consecutive_params: false,
            alignment_consecutive_properties: false,
            alignment_doc_comments: false,
            islands_js: IslandPreset::Oxc,
            islands_css: IslandPreset::Oxc,
            islands_json: IslandPreset::Oxc,
            islands_interpolated: true,
            islands_config: IslandConfigMode::Auto,
        }
    }
}

/// `newline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NewlineStyle {
    /// The platform's line ending (`"os"`).
    #[default]
    #[serde(rename = "os")]
    Os,
    /// `"\n"`.
    #[serde(rename = "\n")]
    Lf,
    /// `"\r\n"`.
    #[serde(rename = "\r\n")]
    CrLf,
    /// The input's dominant line ending (`"auto"`).
    #[serde(rename = "auto")]
    Auto,
}

/// `strings.quote`, `strings.attributes.quote`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuoteStyle {
    /// `'…'`
    Single,
    /// `"…"`
    Double,
    /// Leave quotes as written.
    Ignored,
}

/// `strings.convert_nested_quotes`: the quote of a string whose text holds
/// a quote character (quotes inside `#…#` are code and do not count). The
/// key took `true` / `false` before it took strings; those still load, as
/// `"always"` / `"never"`, with a warning ([`Options::migrate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NestedQuotes {
    /// `"always"`: the configured quote, doubled inside the text.
    Always,
    /// `"never"`: the quote as written.
    Never,
    /// `"fewer_escapes"`: the configured quote unless the text holds more of
    /// it than of the other, then the other (Prettier's rule).
    #[default]
    FewerEscapes,
}

/// `multiline.comma`: the commas of a delimited list printed one item per
/// line (structs, arrays, arguments, parameters, script-tag attributes). A
/// list printed on one line is `a, b` under every style. The two dangling
/// values differ by list: [`CommaStyle::literal`] and [`CommaStyle::list`]
/// give the style each kind of list prints with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommaStyle {
    /// `a,` newline `b`: no comma after the last item.
    #[default]
    Trailing,
    /// `a,` newline `b,`: a comma after the last item too, in struct and
    /// array literals only; argument and parameter lists print
    /// [`CommaStyle::Trailing`]. Lucee rejects a comma after the last
    /// argument of a call.
    Dangling,
    /// A comma after the last item in every list: struct and array
    /// literals, arguments and parameters.
    DanglingAll,
    /// `a` newline `, b`, the first item spaced by two so the items align.
    Leading,
    /// `a` newline `,b`, the first item spaced by one.
    LeadingTight,
}

impl CommaStyle {
    /// The JSON value of the style.
    pub fn as_str(self) -> &'static str {
        match self {
            CommaStyle::Trailing => "trailing",
            CommaStyle::Dangling => "dangling",
            CommaStyle::DanglingAll => "dangling_all",
            CommaStyle::Leading => "leading",
            CommaStyle::LeadingTight => "leading_tight",
        }
    }

    /// The style a struct literal (`{…}`, `[a: 1]`) or an array literal
    /// prints with: both dangling values dangle. Never
    /// [`CommaStyle::DanglingAll`].
    pub fn literal(self) -> CommaStyle {
        match self {
            CommaStyle::DanglingAll => CommaStyle::Dangling,
            style => style,
        }
    }

    /// The style an argument list (calls, `new`, script-tag calls) or a
    /// parameter list (named, anonymous and arrow functions) prints with:
    /// only [`CommaStyle::DanglingAll`] dangles. Never
    /// [`CommaStyle::DanglingAll`].
    pub fn list(self) -> CommaStyle {
        match self {
            CommaStyle::Dangling => CommaStyle::Trailing,
            CommaStyle::DanglingAll => CommaStyle::Dangling,
            style => style,
        }
    }
}

/// `function_call.casing.builtin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuiltinCasing {
    /// The casing used by cfdocs.org.
    Cfdocs,
    /// `ArrayAppend`
    Pascal,
    /// As written.
    Ignored,
}

/// `function_call.casing.userdefined`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserDefinedCasing {
    /// As written.
    Ignored,
    /// `myFunc`
    Camel,
    /// `MyFunc`
    Pascal,
}

/// `comment.asterisks`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Asterisks {
    /// Leading `*` one column right of the comment's indent.
    Align,
    /// Leading `*` at the comment's indent.
    Indent,
    /// Print the comment verbatim.
    Ignored,
}

/// `tags.body.indent`: how a broken paired CF tag body is indented.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagBodyIndent {
    /// Every broken tag body one level in.
    #[default]
    Always,
    /// A paired CF tag whose body's first significant node is not CFML (a
    /// CF tag) prints that body at the tag's own indent.
    Cfml,
}

/// `islands.js` / `islands.css` / `islands.json`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IslandPreset {
    /// Formatted in process by the oxc formatter crates.
    #[default]
    Oxc,
    /// Islands print verbatim.
    Off,
}

/// By hand, so that every wrong value (another string, the array or number
/// of an older setting) names the two accepted strings.
impl<'de> Deserialize<'de> for IslandPreset {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = IslandPreset;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("`oxc` or `off`")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<IslandPreset, E> {
                match v {
                    "oxc" => Ok(IslandPreset::Oxc),
                    "off" => Ok(IslandPreset::Off),
                    _ => Err(E::unknown_variant(v, &["oxc", "off"])),
                }
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// `islands.config`: whether the `"oxc"` formatter reads the project's
/// formatter configuration (`crate::islands` discovers it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IslandConfigMode {
    /// `.oxfmtrc.json` / `.oxfmtrc.jsonc`, or prettier's `.prettierrc`
    /// family, from the file's directory upward.
    #[default]
    Auto,
    /// oxc's defaults.
    Off,
}

/// A migration note about a key of the input settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Warning {
    /// The key as written in the settings.
    pub key: String,
    /// What happened to it.
    pub message: String,
}

impl Warning {
    /// Whether this is the warning of per-construct comma keys that no
    /// `multiline.comma` value reproduces, where the migration chose one:
    /// rewriting the file silences it, so `cfformat settings --migrate`
    /// names the value chosen.
    pub fn chose_comma(&self) -> bool {
        self.message.starts_with(COMMA_CHOICE)
    }
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}`: {}", self.key, self.message)
    }
}

/// Settings that cannot be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionsError {
    /// Not JSON, not an object, an unknown key or a value of the wrong type.
    Invalid(String),
    /// A settings file that cannot be read or holds invalid settings.
    File {
        /// The settings file.
        path: PathBuf,
        /// What is wrong with it.
        message: String,
    },
}

impl fmt::Display for OptionsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OptionsError::Invalid(message) => write!(f, "invalid settings: {message}"),
            OptionsError::File { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}

impl std::error::Error for OptionsError {}

/// CommandBox cfformat keys that are removed, with the behaviour that
/// replaced them.
const REMOVED: &[(&str, &str)] = &[
    (
        "struct.multiline.min_length",
        "the threshold measures the items' average width instead: `struct.multiline.min_item_length`",
    ),
    (
        "array.multiline.min_length",
        "the threshold measures the items' average width instead: `array.multiline.min_item_length`",
    ),
    (
        "function_call.multiline.min_length",
        "the threshold measures the items' average width instead: `function_call.multiline.min_item_length`",
    ),
    (
        "function_declaration.multiline.min_length",
        "the threshold measures the items' average width instead: `function_declaration.multiline.min_item_length`",
    ),
    (
        "function_anonymous.multiline.min_length",
        "the threshold measures the items' average width instead: `function_anonymous.multiline.min_item_length`",
    ),
    (
        "metadata.multiline.min_length",
        "the threshold measures the items' average width instead: `metadata.multiline.min_item_length`",
    ),
    (
        "param.multiline.min_length",
        "the threshold measures the items' average width instead: `param.multiline.min_item_length`",
    ),
    (
        "property.multiline.min_length",
        "the threshold measures the items' average width instead: `property.multiline.min_item_length`",
    ),
    (
        "binary_operators.padding",
        "binary operators are always padded (`a + b`)",
    ),
    (
        "binary_operators.newline_indent",
        "a broken binary expression always indents its continuation lines once",
    ),
    (
        "for_loop_semicolons.padding",
        "`for` headers always print `for (a; b; c)`",
    ),
    (
        "keywords.block_to_keyword_spacing",
        "the formatter always prints `} else {`",
    ),
    (
        "keywords.group_to_block_spacing",
        "the formatter always prints `if (x) {`",
    ),
    (
        "keywords.spacing_to_block",
        "the formatter always prints `do {`",
    ),
    (
        "keywords.spacing_to_group",
        "the formatter always prints `if (`",
    ),
    (
        "keywords.padding_inside_group",
        "keyword groups follow `parentheses.padding`",
    ),
    (
        "keywords.empty_group_spacing",
        "empty groups always print `()`",
    ),
    (
        "function_declaration.group_to_block_spacing",
        "the formatter always prints `function f() {`",
    ),
    (
        "function_anonymous.group_to_block_spacing",
        "the formatter always prints `function() {`",
    ),
    (
        "function_declaration.spacing_to_group",
        "the formatter always prints `function f(`",
    ),
    (
        "function_anonymous.spacing_to_group",
        "the formatter always prints `function(`",
    ),
    (
        "function_call.empty_padding",
        "an empty argument list always prints `()`",
    ),
    (
        "function_declaration.empty_padding",
        "an empty parameter list always prints `()`",
    ),
    (
        "function_anonymous.empty_padding",
        "an empty parameter list always prints `()`",
    ),
];

/// CommandBox cfformat keys renamed or merged: old name, new name, and
/// whether the rename is only a spelling change (accepted silently).
const RENAMED: &[(&str, &str, bool)] = &[
    (
        "strings.convertNestedQuotes",
        "strings.convert_nested_quotes",
        true,
    ),
    ("function_call.padding", "parentheses.padding", false),
    ("function_declaration.padding", "parentheses.padding", false),
    ("function_anonymous.padding", "parentheses.padding", false),
    (
        "metadata.key_value.padding",
        "attributes.key_value.padding",
        false,
    ),
    (
        "param.key_value.padding",
        "attributes.key_value.padding",
        false,
    ),
    (
        "property.key_value.padding",
        "attributes.key_value.padding",
        false,
    ),
];

/// The constructs whose `*.multiline.comma_dangle`,
/// `*.multiline.leading_comma` and `*.multiline.leading_comma.padding` keys
/// `multiline.comma` replaces.
const COMMA_CONSTRUCTS: &[&str] = &[
    "struct",
    "array",
    "function_call",
    "function_declaration",
    "function_anonymous",
];

/// One construct's old comma keys, as found in a settings object.
struct OldCommas {
    construct: &'static str,
    /// The first of its keys in the input.
    first_key: String,
    dangle: Option<bool>,
    leading: Option<bool>,
    padding: Option<bool>,
}

impl OldCommas {
    /// The style the trio asked for (CommandBox's rules: leading commas never
    /// dangle, their padding defaults to true); `None` when neither
    /// `comma_dangle` nor `leading_comma` is set — a lone `.padding` key did
    /// nothing.
    fn resolve(&self, warnings: &mut Vec<Warning>) -> Option<CommaStyle> {
        if self.leading.is_none() && self.dangle.is_none() {
            return None;
        }
        Some(if self.leading == Some(true) {
            if self.dangle == Some(true) {
                warnings.push(Warning {
                    key: format!("{}.multiline.comma_dangle", self.construct),
                    message: format!(
                        "is ignored with leading commas (`{}.multiline.leading_comma`)",
                        self.construct
                    ),
                });
            }
            if self.padding.unwrap_or(true) {
                CommaStyle::Leading
            } else {
                CommaStyle::LeadingTight
            }
        } else if self.dangle == Some(true) {
            CommaStyle::Dangling
        } else {
            CommaStyle::Trailing
        })
    }
}

/// Moves the per-construct comma keys into `multiline.comma`: each
/// construct's trio resolves to one style ([`OldCommas::resolve`]), then the
/// set of styles to one value ([`merge_commas`]). `multiline.comma` itself,
/// when present, wins over every trio, and a trio whose construct it gives
/// another style warns "ignored".
fn migrate_commas(map: Map<String, Value>, warnings: &mut Vec<Warning>) -> Map<String, Value> {
    let mut out = Map::new();
    let mut found: Vec<OldCommas> = Vec::new();
    for (key, value) in map {
        let field = COMMA_CONSTRUCTS.iter().find_map(|&c| {
            let field = key.strip_prefix(c)?.strip_prefix(".multiline.")?;
            matches!(
                field,
                "comma_dangle" | "leading_comma" | "leading_comma.padding"
            )
            .then_some((c, field.to_owned()))
        });
        let Some((construct, field)) = field else {
            out.insert(key, value);
            continue;
        };
        let Some(flag) = value.as_bool() else {
            warnings.push(Warning {
                key,
                message: "expects true or false; ignored".into(),
            });
            continue;
        };
        let at = match found.iter().position(|o| o.construct == construct) {
            Some(at) => at,
            None => {
                found.push(OldCommas {
                    construct,
                    first_key: key.clone(),
                    dangle: None,
                    leading: None,
                    padding: None,
                });
                found.len() - 1
            }
        };
        let old = &mut found[at];
        match field.as_str() {
            "comma_dangle" => old.dangle = Some(flag),
            "leading_comma" => old.leading = Some(flag),
            _ => old.padding = Some(flag),
        }
    }
    let resolved: Vec<(&OldCommas, CommaStyle)> = found
        .iter()
        .filter_map(|o| Some((o, o.resolve(warnings)?)))
        .collect();
    if let Some(set) = out.get(NEW_COMMA_KEY) {
        let set: Option<CommaStyle> = serde_json::from_value(set.clone()).ok();
        for (old, style) in &resolved {
            if set.map(|set| applied(old.construct, set)) != Some(*style) {
                warnings.push(Warning {
                    key: old.first_key.clone(),
                    message: format!(
                        "is replaced by `{NEW_COMMA_KEY}`, which is already set; \"{}\" ignored",
                        style.as_str()
                    ),
                });
            }
        }
        return out;
    }
    if let Some(value) = merge_commas(&resolved, warnings) {
        out.insert(NEW_COMMA_KEY.into(), Value::from(value.as_str()));
    }
    out
}

/// Whether a construct of [`COMMA_CONSTRUCTS`] is a struct or array literal
/// (the rest are argument and parameter lists).
fn is_literal(construct: &str) -> bool {
    matches!(construct, "struct" | "array")
}

/// The style `value` gives `construct`'s lists: [`CommaStyle::literal`] or
/// [`CommaStyle::list`].
fn applied(construct: &str, value: CommaStyle) -> CommaStyle {
    if is_literal(construct) {
        value.literal()
    } else {
        value.list()
    }
}

/// The `multiline.comma` value that gives `construct` `style`: a dangling
/// argument or parameter list needs `"dangling_all"`.
fn value_for(construct: &str, style: CommaStyle) -> CommaStyle {
    if style == CommaStyle::Dangling && !is_literal(construct) {
        CommaStyle::DanglingAll
    } else {
        style
    }
}

/// The one `multiline.comma` value for the constructs' resolved styles, in
/// file order; `None` when no construct set a key. With a leading style
/// among them, they must all agree. Otherwise every construct trailing is
/// `"trailing"`; every construct dangling is `"dangling_all"` when an
/// argument or parameter list is among them, else `"dangling"`; struct and
/// array literals dangling with argument and parameter lists trailing is
/// `"dangling"`. Anything else no value expresses: one warning, on the first
/// construct that does not get its style, and the first construct decides.
fn merge_commas(
    resolved: &[(&OldCommas, CommaStyle)],
    warnings: &mut Vec<Warning>,
) -> Option<CommaStyle> {
    let &(first, first_style) = resolved.first()?;
    let chosen = value_for(first.construct, first_style);
    let all = |style: CommaStyle| resolved.iter().all(|&(_, s)| s == style);
    let leading = resolved
        .iter()
        .any(|&(_, s)| matches!(s, CommaStyle::Leading | CommaStyle::LeadingTight));
    let merged = if leading {
        all(first_style).then_some(first_style)
    } else if all(CommaStyle::Trailing) {
        Some(CommaStyle::Trailing)
    } else if all(CommaStyle::Dangling) {
        Some(if resolved.iter().any(|(o, _)| !is_literal(o.construct)) {
            CommaStyle::DanglingAll
        } else {
            CommaStyle::Dangling
        })
    } else if resolved
        .iter()
        .all(|&(o, s)| s == applied(o.construct, CommaStyle::Dangling))
    {
        Some(CommaStyle::Dangling)
    } else {
        None
    };
    if let Some(value) = merged {
        return Some(value);
    }
    let loser = resolved
        .iter()
        .find(|&&(o, s)| applied(o.construct, chosen) != s)
        .map_or(first, |&(o, _)| o);
    let styles: Vec<String> = resolved
        .iter()
        .map(|(o, style)| format!("{} \"{}\"", o.construct, style.as_str()))
        .collect();
    warnings.push(Warning {
        key: loser.first_key.clone(),
        message: format!(
            "{COMMA_CHOICE} which cannot give each of these constructs its own style; they \
             disagree ({}), so the first decides: \"{}\"",
            styles.join(", "),
            chosen.as_str()
        ),
    });
    Some(chosen)
}

/// How the warning of a comma disagreement begins ([`Warning::chose_comma`]).
const COMMA_CHOICE: &str = "is now `multiline.comma`,";

/// The key that replaces the per-construct comma keys.
const NEW_COMMA_KEY: &str = "multiline.comma";

/// Keys that took a boolean and now take a string: the key, then the
/// strings `true` and `false` become.
const BOOLEAN_VALUES: &[(&str, &str, &str)] =
    &[("strings.convert_nested_quotes", "always", "never")];

/// Rewrites the boolean value of a [`BOOLEAN_VALUES`] key, under its current
/// name or an old one, to its string, with a warning naming the key as
/// written. Any other value is left for deserialisation to judge.
fn migrate_booleans(map: &mut Map<String, Value>, warnings: &mut Vec<Warning>) {
    for (key, value) in map.iter_mut() {
        let name = RENAMED
            .iter()
            .find(|(old, _, _)| old == key)
            .map_or(key.as_str(), |&(_, new, _)| new);
        let Some(&(_, yes, no)) = BOOLEAN_VALUES.iter().find(|(k, _, _)| *k == name) else {
            continue;
        };
        let Some(flag) = value.as_bool() else {
            continue;
        };
        let string = if flag { yes } else { no };
        warnings.push(Warning {
            key: key.clone(),
            message: format!("`{flag}` is now `\"{string}\"`"),
        });
        *value = Value::from(string);
    }
}

impl Options {
    /// Rewrites a raw settings object for the current key set: removed keys
    /// are dropped with a warning, renamed keys move to their new name (a
    /// rename that is only a spelling change is silent). When the new key is
    /// already present, or several old keys merge into one, the first value
    /// seen wins — the new key itself first, then old keys in input order —
    /// and every other one is dropped with a warning. The per-construct
    /// comma keys (`*.multiline.comma_dangle`, `*.multiline.leading_comma`,
    /// `*.multiline.leading_comma.padding`) become one `multiline.comma`
    /// value: constructs that agree merge silently, as do struct and array
    /// literals dangling with argument and parameter lists trailing
    /// (`"dangling"`); any other disagreement warns once and the first
    /// construct seen decides; `multiline.comma` itself wins over all of
    /// them. A boolean `strings.convert_nested_quotes` (under
    /// either name) becomes `"always"` or `"never"` with a warning. Unknown
    /// keys are left for deserialisation to reject.
    pub fn migrate(map: Map<String, Value>) -> (Map<String, Value>, Vec<Warning>) {
        let mut warnings = Vec::new();
        let mut map = migrate_commas(map, &mut warnings);
        migrate_booleans(&mut map, &mut warnings);
        let mut out = Map::new();
        let renamed: Vec<(String, Value)> = map
            .into_iter()
            .filter_map(|(key, value)| {
                if let Some((_, why)) = REMOVED.iter().find(|(k, _)| *k == key) {
                    warnings.push(Warning {
                        message: format!("is no longer configurable; {why}"),
                        key,
                    });
                    return None;
                }
                if RENAMED.iter().any(|(old, _, _)| *old == key) {
                    return Some((key, value));
                }
                out.insert(key, value);
                None
            })
            .collect();
        for (old, value) in renamed {
            let &(_, new, spelling_only) = RENAMED.iter().find(|(o, _, _)| *o == old).unwrap();
            if out.contains_key(new) {
                warnings.push(Warning {
                    message: format!("is replaced by `{new}`, which is already set; ignored"),
                    key: old,
                });
                continue;
            }
            if !spelling_only {
                warnings.push(Warning {
                    message: format!("is now `{new}`"),
                    key: old,
                });
            }
            out.insert(new.to_string(), value);
        }
        (out, warnings)
    }

    /// Migrates ([`Options::migrate`]), deserialises and validates a
    /// settings object ([`Options::validate`]).
    pub fn from_map(map: Map<String, Value>) -> Result<(Options, Vec<Warning>), OptionsError> {
        let (map, warnings) = Options::migrate(map);
        Ok((Options::validate(&map)?, warnings))
    }

    /// Deserialises and validates a settings object that is already
    /// migrated: an unknown key or a value of the wrong type is an error
    /// naming the key, `struct.separator` is `:` or `=` with at most one
    /// space on each side, and `indent_size` and `max_columns` are at least
    /// 1. The error is always [`OptionsError::Invalid`].
    pub fn validate(map: &Map<String, Value>) -> Result<Options, OptionsError> {
        validated(map).map_err(OptionsError::Invalid)
    }

    /// Parses a `.cfformat.json` document (one flat object).
    pub fn from_json(json: &str) -> Result<(Options, Vec<Warning>), OptionsError> {
        match serde_json::from_str(json).map_err(|e| OptionsError::Invalid(e.to_string()))? {
            Value::Object(map) => Options::from_map(map),
            _ => Err(OptionsError::Invalid("expected a JSON object".into())),
        }
    }

    /// The indentation, `tab_indent` and `indent_size` together: the
    /// document printer's, and an island's.
    pub fn indent_style(&self) -> cfdoc::IndentStyle {
        if self.tab_indent {
            cfdoc::IndentStyle::Tabs(self.indent_size)
        } else {
            cfdoc::IndentStyle::Spaces(self.indent_size)
        }
    }

    /// The printer's line ending; `tree_newline` answers `"auto"`.
    pub fn newline_str(&self, tree_newline: cfparse::Newline) -> &'static str {
        match self.newline {
            NewlineStyle::Os if cfg!(windows) => "\r\n",
            NewlineStyle::Os | NewlineStyle::Lf => "\n",
            NewlineStyle::CrLf => "\r\n",
            NewlineStyle::Auto => tree_newline.as_str(),
        }
    }
}

/// [`Options::validate`], its error the message alone.
fn validated(map: &Map<String, Value>) -> Result<Options, String> {
    let options = Options::deserialize(map).map_err(|e| invalid_key(map, &e))?;
    let separator = options.struct_separator.as_str();
    let core = separator.strip_prefix(' ').unwrap_or(separator);
    let core = core.strip_suffix(' ').unwrap_or(core);
    if core != ":" && core != "=" {
        return Err(format!(
            "`struct.separator` must be `:` or `=` with at most one space on each side, not {separator:?}"
        ));
    }
    for (key, value) in [
        ("indent_size", options.indent_size),
        ("max_columns", options.max_columns),
    ] {
        if value < 1 {
            return Err(format!("`{key}` must be at least 1"));
        }
    }
    Ok(options)
}

/// A deserialisation error's message naming the key it is about (serde's
/// message for a bad value names the value only): the first key that fails
/// on its own.
fn invalid_key(map: &Map<String, Value>, error: &serde_json::Error) -> String {
    let message = error.to_string();
    let key = map.iter().find_map(|(key, value)| {
        let one = Map::from_iter([(key.clone(), value.clone())]);
        Options::deserialize(&one).is_err().then_some(key)
    });
    match key {
        Some(key) if !message.contains(&format!("`{key}`")) => format!("`{key}`: {message}"),
        _ => message,
    }
}

/// The name of a settings file.
pub const SETTINGS_FILE: &str = ".cfformat.json";

/// A settings file as read once per run: its migrated object and the
/// migration warnings it produced.
#[derive(Debug)]
struct Layer {
    map: Map<String, Value>,
    warnings: Vec<Warning>,
}

/// The options of one file and the settings files they came from.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The merged, validated options.
    pub options: Options,
    /// The settings files that were merged, in merge order (later wins): at
    /// most the discovered file, then the explicit config file.
    pub sources: Vec<PathBuf>,
    /// Migration warnings, each with the settings file it came from.
    pub warnings: Vec<(PathBuf, Warning)>,
}

/// Settings discovery, first found wins: the discovered file is not layered
/// over the home file, as it was in CommandBox cfformat. For a file, the
/// nearest `.cfformat.json` walking up from the file's directory — the walk
/// stops after the first directory that contains `.git` — is the only
/// discovered settings file; only when the walk finds none is the home file
/// used, when it exists. The explicit config file is merged over either, key
/// by key, and the result deserialised once.
///
/// A `Discovery` caches, per directory, which settings file applies, and per
/// settings file its parsed object and warnings, so a run over a tree reads
/// each settings file once — however many threads ask for it at the same
/// time, and a file that fails to load fails once: its error is kept. It is
/// `Sync`.
#[derive(Debug)]
pub struct Discovery {
    home: Option<PathBuf>,
    config: Option<PathBuf>,
    dirs: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
    files: Mutex<HashMap<PathBuf, Arc<LayerCell>>>,
    /// How many settings files were read.
    #[cfg(test)]
    reads: std::sync::atomic::AtomicUsize,
}

/// One settings file's load, done once by whichever thread gets there first
/// while the others wait on it.
type LayerCell = OnceLock<Result<Arc<Layer>, OptionsError>>;

impl Discovery {
    /// Discovery with a home settings file, the fallback when a walk finds no
    /// settings file (`None` disables it; see [`Discovery::home_file`]), and
    /// an explicit config file, which must exist when given.
    pub fn new(home: Option<PathBuf>, config: Option<PathBuf>) -> Self {
        Discovery {
            home,
            config,
            dirs: Mutex::default(),
            files: Mutex::default(),
            #[cfg(test)]
            reads: Default::default(),
        }
    }

    /// `$HOME/.cfformat.json` (`%USERPROFILE%` on Windows), whether or not it
    /// exists; `None` when the variable is unset.
    pub fn home_file() -> Option<PathBuf> {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        std::env::var_os(var)
            .filter(|h| !h.is_empty())
            .map(|h| PathBuf::from(h).join(SETTINGS_FILE))
    }

    /// The options for `file`: the walk starts at its directory.
    pub fn discover(&self, file: &Path) -> Result<Resolved, OptionsError> {
        let file = absolute(file);
        let dir = file.parent().unwrap_or(&file);
        self.discover_in(dir)
    }

    /// The options for a file inside `dir` (a directory argument, or the
    /// current directory for stdin).
    pub fn discover_in(&self, dir: &Path) -> Result<Resolved, OptionsError> {
        let dir = absolute(dir);
        let mut layers: Vec<PathBuf> = Vec::new();
        // A walk that passed through the home directory checked the home file
        // there and found none, so the fallback cannot read a file twice.
        if let Some(nearest) = self
            .nearest(&dir)
            .or_else(|| self.home.clone().filter(|h| h.is_file()))
        {
            layers.push(nearest);
        }
        if let Some(config) = &self.config {
            layers.push(absolute(config));
        }
        let mut merged = Map::new();
        let mut warnings = Vec::new();
        for path in &layers {
            let layer = self.layer(path)?;
            for (key, value) in &layer.map {
                merged.insert(key.clone(), value.clone());
            }
            warnings.extend(layer.warnings.iter().map(|w| (path.clone(), w.clone())));
        }
        // Every layer is migrated and valid on its own (`layer`), so the
        // merge needs no migration and is valid too.
        let options = Options::validate(&merged)?;
        Ok(Resolved {
            options,
            sources: layers,
            warnings,
        })
    }

    /// The nearest settings file from `dir` upwards; every
    /// directory checked on the way is cached with the answer.
    fn nearest(&self, dir: &Path) -> Option<PathBuf> {
        let mut cache = self.dirs.lock().unwrap_or_else(|e| e.into_inner());
        let mut checked = Vec::new();
        let mut found = None;
        let mut next = Some(dir);
        while let Some(d) = next {
            if let Some(hit) = cache.get(d) {
                found = hit.clone();
                break;
            }
            checked.push(d.to_path_buf());
            let candidate = d.join(SETTINGS_FILE);
            if candidate.is_file() {
                found = Some(candidate);
                break;
            }
            // A `.git` directory, or the `.git` file of a worktree.
            if d.join(".git").exists() {
                break;
            }
            next = d.parent();
        }
        for d in checked {
            cache.insert(d, found.clone());
        }
        found
    }

    /// One settings file, read, migrated and validated once: the cell is
    /// taken under the lock, the load runs outside it.
    fn layer(&self, path: &Path) -> Result<Arc<Layer>, OptionsError> {
        let cell = Arc::clone(
            self.files
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(path.to_path_buf())
                .or_default(),
        );
        cell.get_or_init(|| {
            #[cfg(test)]
            self.reads
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Discovery::load(path)
        })
        .clone()
    }

    /// Reads, migrates and validates one settings file.
    fn load(path: &Path) -> Result<Arc<Layer>, OptionsError> {
        let error = |message: String| OptionsError::File {
            path: path.to_path_buf(),
            message,
        };
        let json = std::fs::read_to_string(path).map_err(|e| error(e.to_string()))?;
        let map = match serde_json::from_str(&json).map_err(|e| error(e.to_string()))? {
            Value::Object(map) => map,
            _ => return Err(error("expected a JSON object".into())),
        };
        let (map, warnings) = Options::migrate(map);
        validated(&map).map_err(error)?;
        Ok(Arc::new(Layer { map, warnings }))
    }
}

/// `path` made absolute against the current directory (not canonicalised).
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(json: &str) -> Map<String, Value> {
        match serde_json::from_str(json).unwrap() {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    /// Pins the shipped defaults, and that an empty settings file gives them.
    #[test]
    fn shipped_defaults() {
        let o = Options::default();
        assert_eq!(
            (o.indent_size, o.tab_indent, o.max_columns),
            (4, false, 120)
        );
        assert_eq!(o.newline, NewlineStyle::Os);
        assert_eq!(o.strings_quote, QuoteStyle::Single);
        assert_eq!(o.strings_attributes_quote, QuoteStyle::Double);
        assert_eq!(o.strings_convert_nested_quotes, NestedQuotes::FewerEscapes);
        assert_eq!(o.struct_separator, ": ");
        assert_eq!(o.comment_asterisks, Asterisks::Align);
        assert_eq!(o.method_call_chain_multiline, 0);
        assert_eq!(o.function_call_casing_builtin, BuiltinCasing::Cfdocs);
        for key in [o.islands_js, o.islands_css, o.islands_json] {
            assert_eq!(key, IslandPreset::Oxc);
        }
        let (parsed, warnings) = Options::from_json("{}").unwrap();
        assert_eq!(parsed, o);
        assert!(warnings.is_empty());
    }

    #[test]
    fn every_commandbox_key_is_known() {
        // The 77 keys of commandbox-cfformat's `.cfformat.json`: a warning
        // per removed key and per renamed key but the silent spelling
        // change, whose boolean value warns instead.
        let keys = include_str!("../tests/data/commandbox-cfformat.json");
        let (_, warnings) = Options::from_json(keys).unwrap();
        assert_eq!(warnings.len(), REMOVED.len() + RENAMED.len());
    }

    #[test]
    fn attribute_alignment_keys_load() {
        // CommandBox's attribute alignments are options again: a file naming
        // them loads as it is, with nothing to migrate.
        let json =
            r#"{"alignment.consecutive.params": true, "alignment.consecutive.properties": true}"#;
        let (o, w) = Options::from_json(json).unwrap();
        assert!(o.alignment_consecutive_params && o.alignment_consecutive_properties);
        assert!(w.is_empty(), "{w:?}");
        let (m, w) = Options::migrate(map(json));
        assert_eq!(m, map(json));
        assert!(w.is_empty(), "{w:?}");
        let (o, _) = Options::from_json("{}").unwrap();
        assert!(!o.alignment_consecutive_params && !o.alignment_consecutive_properties);
    }

    #[test]
    fn renamed_key_moves() {
        let (o, w) = Options::from_json(r#"{"strings.convertNestedQuotes": "never"}"#).unwrap();
        assert_eq!(o.strings_convert_nested_quotes, NestedQuotes::Never);
        assert!(w.is_empty(), "a spelling change is silent: {w:?}");

        let (o, w) = Options::from_json(r#"{"function_call.padding": true}"#).unwrap();
        assert!(o.parentheses_padding);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, "function_call.padding");
        assert!(w[0].message.contains("parentheses.padding"));
    }

    #[test]
    fn new_key_wins_over_old() {
        let (m, w) = Options::migrate(map(
            r#"{"function_call.padding": true, "parentheses.padding": false}"#,
        ));
        assert_eq!(m["parentheses.padding"], Value::Bool(false));
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn nested_quotes_values() {
        for (value, style) in [
            ("always", NestedQuotes::Always),
            ("never", NestedQuotes::Never),
            ("fewer_escapes", NestedQuotes::FewerEscapes),
        ] {
            let json = format!(r#"{{"strings.convert_nested_quotes": "{value}"}}"#);
            let (o, w) = Options::from_json(&json).unwrap();
            assert_eq!((o.strings_convert_nested_quotes, w), (style, vec![]));
            assert_eq!(
                serde_json::to_value(o).unwrap()["strings.convert_nested_quotes"],
                Value::from(value)
            );
        }
        for bad in [r#""true""#, r#""nope""#, "1", "null"] {
            let json = format!(r#"{{"strings.convert_nested_quotes": {bad}}}"#);
            let err = Options::from_json(&json).unwrap_err().to_string();
            assert!(err.contains("`strings.convert_nested_quotes`"), "{err}");
        }
    }

    #[test]
    fn nested_quotes_booleans_migrate() {
        // The booleans the key took before it took strings load as the
        // strings they meant, with a warning naming the key as written.
        for (key, flag, value, style) in [
            (
                "strings.convert_nested_quotes",
                "true",
                "always",
                NestedQuotes::Always,
            ),
            (
                "strings.convert_nested_quotes",
                "false",
                "never",
                NestedQuotes::Never,
            ),
            (
                "strings.convertNestedQuotes",
                "true",
                "always",
                NestedQuotes::Always,
            ),
            (
                "strings.convertNestedQuotes",
                "false",
                "never",
                NestedQuotes::Never,
            ),
        ] {
            let json = format!(r#"{{"{key}": {flag}}}"#);
            let (m, w) = Options::migrate(map(&json));
            assert_eq!(
                m,
                map(&format!(
                    r#"{{"strings.convert_nested_quotes": "{value}"}}"#
                ))
            );
            assert_eq!(
                w,
                vec![Warning {
                    key: key.into(),
                    message: format!("`{flag}` is now `\"{value}\"`"),
                }]
            );
            let (o, _) = Options::from_json(&json).unwrap();
            assert_eq!(o.strings_convert_nested_quotes, style);
        }
        // Validation alone (an object already migrated) takes no boolean.
        let err = Options::validate(&map(r#"{"strings.convert_nested_quotes": true}"#))
            .unwrap_err()
            .to_string();
        assert!(err.contains("`strings.convert_nested_quotes`"), "{err}");
    }

    /// `multiline.comma` and the warnings after migrating `json`.
    fn commas(json: &str) -> (Option<String>, Vec<Warning>) {
        let (m, w) = Options::migrate(map(json));
        let comma = m
            .get("multiline.comma")
            .map(|v| v.as_str().unwrap().to_owned());
        (comma, w)
    }

    #[test]
    fn comma_keys_that_agree_merge_silently() {
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "array.multiline.comma_dangle": true,
                "function_call.multiline.comma_dangle": true,
                "function_declaration.multiline.comma_dangle": true,
                "function_anonymous.multiline.comma_dangle": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling_all"), vec![]));
        // CommandBox's own defaults: every trio false, the padding true.
        let (comma, w) = commas(
            r#"{"struct.multiline.leading_comma": false,
                "struct.multiline.leading_comma.padding": true,
                "struct.multiline.comma_dangle": false,
                "array.multiline.leading_comma": false}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("trailing"), vec![]));
        // A lone padding key did nothing and is dropped silently.
        let (comma, w) = commas(r#"{"array.multiline.leading_comma.padding": false}"#);
        assert_eq!((comma, w), (None, vec![]));
    }

    #[test]
    fn comma_keys_that_disagree_warn_once() {
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "array.multiline.comma_dangle": false,
                "function_call.multiline.comma_dangle": false}"#,
        );
        assert_eq!(comma.as_deref(), Some("dangling"), "the first decides");
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "array.multiline.comma_dangle");
        assert!(w[0].chose_comma(), "{}", w[0]);
        assert_eq!(
            w[0].to_string(),
            "`array.multiline.comma_dangle`: is now `multiline.comma`, which cannot give each \
             of these constructs its own style; they disagree (struct \"dangling\", array \
             \"trailing\", function_call \"trailing\"), so the first decides: \"dangling\""
        );
    }

    /// The rules that merge the constructs' styles ([`merge_commas`]), by
    /// group: struct and array literals, argument and parameter lists.
    #[test]
    fn comma_keys_merge_by_group() {
        // Literals dangling, lists trailing (Prettier's "es5"): "dangling",
        // which is exactly that, silently.
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "array.multiline.comma_dangle": true,
                "function_call.multiline.comma_dangle": false,
                "function_declaration.multiline.comma_dangle": false,
                "function_anonymous.multiline.comma_dangle": false}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling"), vec![]));
        // The same with the lists first in the file.
        let (comma, w) = commas(
            r#"{"function_call.multiline.comma_dangle": false,
                "array.multiline.comma_dangle": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling"), vec![]));
        // Every construct set dangling: "dangling_all" once a list is among
        // them, "dangling" for literals alone.
        let (comma, w) = commas(r#"{"function_call.multiline.comma_dangle": true}"#);
        assert_eq!((comma.as_deref(), w), (Some("dangling_all"), vec![]));
        let (comma, w) = commas(r#"{"struct.multiline.comma_dangle": true}"#);
        assert_eq!((comma.as_deref(), w), (Some("dangling"), vec![]));
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true,
                "function_anonymous.multiline.comma_dangle": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling_all"), vec![]));
        // Every construct set trailing.
        let (comma, w) = commas(
            r#"{"function_call.multiline.comma_dangle": false,
                "struct.multiline.comma_dangle": false}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("trailing"), vec![]));
        // Lists dangling, literals trailing: no value does that; the first
        // construct decides, a list dangling is "dangling_all".
        let (comma, w) = commas(
            r#"{"function_call.multiline.comma_dangle": true,
                "function_declaration.multiline.comma_dangle": true,
                "struct.multiline.comma_dangle": false}"#,
        );
        assert_eq!(comma.as_deref(), Some("dangling_all"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "struct.multiline.comma_dangle");
        assert!(w[0].chose_comma(), "{}", w[0]);
        assert!(
            w[0].message.contains(
                "(function_call \"dangling\", function_declaration \"dangling\", \
                 struct \"trailing\"), so the first decides: \"dangling_all\""
            ),
            "{}",
            w[0]
        );
        // The same, literals first: the first is trailing.
        let (comma, w) = commas(
            r#"{"array.multiline.comma_dangle": false,
                "function_call.multiline.comma_dangle": true}"#,
        );
        assert_eq!(comma.as_deref(), Some("trailing"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "function_call.multiline.comma_dangle");
        assert!(w[0].chose_comma(), "{}", w[0]);
        // One list dangling, another trailing.
        let (comma, w) = commas(
            r#"{"function_declaration.multiline.comma_dangle": false,
                "function_call.multiline.comma_dangle": true}"#,
        );
        assert_eq!(comma.as_deref(), Some("trailing"));
        assert_eq!(w[0].key, "function_call.multiline.comma_dangle");
        // A leading style among them: they must all agree.
        let (comma, w) = commas(
            r#"{"struct.multiline.leading_comma": true,
                "function_call.multiline.leading_comma": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("leading"), vec![]));
        let (comma, w) = commas(
            r#"{"function_call.multiline.comma_dangle": true,
                "struct.multiline.leading_comma": true}"#,
        );
        assert_eq!(comma.as_deref(), Some("dangling_all"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "struct.multiline.leading_comma");
        assert!(w[0].chose_comma(), "{}", w[0]);
        // No other warning is the comma choice.
        let (_, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "struct.multiline.leading_comma": true,
                "multiline.comma": "trailing"}"#,
        );
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().all(|w| !w.chose_comma()), "{w:?}");
    }

    #[test]
    fn leading_comma_keys_resolve() {
        let (comma, w) = commas(r#"{"array.multiline.leading_comma": true}"#);
        assert_eq!((comma.as_deref(), w), (Some("leading"), vec![]));
        let (comma, w) = commas(
            r#"{"array.multiline.leading_comma": true,
                "array.multiline.leading_comma.padding": false}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("leading_tight"), vec![]));
        // Leading commas never dangle: one warning, leading wins.
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "struct.multiline.leading_comma": true}"#,
        );
        assert_eq!(comma.as_deref(), Some("leading"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "struct.multiline.comma_dangle");
        assert!(w[0].message.contains("leading commas"), "{}", w[0]);
        let (o, _) =
            Options::from_json(r#"{"function_declaration.multiline.leading_comma": true}"#)
                .unwrap();
        assert_eq!(o.multiline_comma, CommaStyle::Leading);
    }

    #[test]
    fn the_new_comma_key_wins() {
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "multiline.comma": "dangling",
                "array.multiline.comma_dangle": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling"), vec![]));
        let (comma, w) = commas(
            r#"{"multiline.comma": "leading", "array.multiline.comma_dangle": false,
                "array.multiline.leading_comma": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("leading"), vec![]));
        // A construct is ignored only when the value gives it another style.
        let (comma, w) = commas(
            r#"{"struct.multiline.comma_dangle": true, "multiline.comma": "dangling",
                "function_call.multiline.comma_dangle": false}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling"), vec![]));
        let (comma, w) = commas(
            r#"{"array.multiline.comma_dangle": true, "multiline.comma": "dangling_all",
                "function_declaration.multiline.comma_dangle": true}"#,
        );
        assert_eq!((comma.as_deref(), w), (Some("dangling_all"), vec![]));
        let (comma, w) = commas(
            r#"{"function_call.multiline.comma_dangle": true, "multiline.comma": "dangling"}"#,
        );
        assert_eq!(comma.as_deref(), Some("dangling"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "function_call.multiline.comma_dangle");
        assert!(w[0].message.contains("ignored"), "{}", w[0]);
        let (comma, w) =
            commas(r#"{"struct.multiline.comma_dangle": true, "multiline.comma": "trailing"}"#);
        assert_eq!(comma.as_deref(), Some("trailing"));
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].key, "struct.multiline.comma_dangle");
        assert!(w[0].message.contains("ignored"), "{}", w[0]);
    }

    #[test]
    fn comma_values() {
        for (value, style) in [
            ("trailing", CommaStyle::Trailing),
            ("dangling", CommaStyle::Dangling),
            ("dangling_all", CommaStyle::DanglingAll),
            ("leading", CommaStyle::Leading),
            ("leading_tight", CommaStyle::LeadingTight),
        ] {
            let (o, _) =
                Options::from_json(&format!(r#"{{"multiline.comma": "{value}"}}"#)).unwrap();
            assert_eq!(o.multiline_comma, style);
            assert_eq!(style.as_str(), value);
        }
        let err = Options::from_json(r#"{"multiline.comma": "nope"}"#)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("`multiline.comma`") && err.contains("nope"),
            "{err}"
        );
        let (_, w) = commas(r#"{"array.multiline.comma_dangle": "yes"}"#);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn comma_values_by_list() {
        use CommaStyle::*;
        for (value, literal, list) in [
            (Trailing, Trailing, Trailing),
            (Dangling, Dangling, Trailing),
            (DanglingAll, Dangling, Dangling),
            (Leading, Leading, Leading),
            (LeadingTight, LeadingTight, LeadingTight),
        ] {
            assert_eq!(
                (value.literal(), value.list()),
                (literal, list),
                "{value:?}"
            );
        }
    }

    #[test]
    fn min_length_is_replaced_by_min_item_length() {
        // CommandBox's whole-list width has no exact equivalent in the
        // average item width, so the old key is dropped with a warning that
        // names the new one, and the construct's default applies.
        let (o, w) = Options::from_json(r#"{"array.multiline.min_length": 40}"#).unwrap();
        assert_eq!(o.array_multiline_min_item_length, 8);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, "array.multiline.min_length");
        assert!(w[0].message.contains("`array.multiline.min_item_length`"));
    }

    #[test]
    fn removed_key_warns() {
        let (o, w) =
            Options::from_json(r#"{"keywords.spacing_to_group": false, "max_columns": 80}"#)
                .unwrap();
        assert_eq!(o.max_columns, 80);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].key, "keywords.spacing_to_group");
        assert!(w[0].to_string().contains("no longer configurable"));
    }

    #[test]
    fn unknown_key_is_an_error() {
        let err = Options::from_json(r#"{"max_colums": 80}"#).unwrap_err();
        assert!(err.to_string().contains("max_colums"), "{err}");
        assert!(Options::from_json(r#"{"strings.quote": "backtick"}"#).is_err());
        assert!(Options::from_json("[]").is_err());
    }

    #[test]
    fn islands_values() {
        for (value, preset) in [("oxc", IslandPreset::Oxc), ("off", IslandPreset::Off)] {
            let json = format!(
                r#"{{"islands.js": "{value}", "islands.css": "{value}", "islands.json": "{value}"}}"#
            );
            let (o, _) = Options::from_json(&json).unwrap();
            assert_eq!([o.islands_js, o.islands_css, o.islands_json], [preset; 3]);
            assert_eq!(serde_json::to_value(preset).unwrap(), Value::from(value));
        }
        // The removed values are invalid like any other, and `islands.timeout_ms` is an unknown key.
        let error = |json: &str| Options::from_json(json).unwrap_err().to_string();
        for (value, message) in [
            (
                r#""prettier""#,
                "unknown variant `prettier`, expected `oxc` or `off`",
            ),
            (
                r#""biome""#,
                "unknown variant `biome`, expected `oxc` or `off`",
            ),
            (
                r#""oxfmt""#,
                "unknown variant `oxfmt`, expected `oxc` or `off`",
            ),
            (
                r#"["custom", "fmt", "{path}"]"#,
                "invalid type: sequence, expected `oxc` or `off`",
            ),
        ] {
            for key in ["islands.js", "islands.css", "islands.json"] {
                assert_eq!(
                    error(&format!(r#"{{"{key}": {value}}}"#)),
                    format!("invalid settings: `{key}`: {message}")
                );
            }
        }
        assert_eq!(
            error(r#"{"islands.timeout_ms": 5000}"#),
            error(r#"{"no_such_key": 1}"#).replace("no_such_key", "islands.timeout_ms")
        );
    }

    #[test]
    fn islands_config_values() {
        assert_eq!(Options::default().islands_config, IslandConfigMode::Auto);
        let (o, _) = Options::from_json(r#"{"islands.config": "off"}"#).unwrap();
        assert_eq!(o.islands_config, IslandConfigMode::Off);
        let (o, _) = Options::from_json(r#"{"islands.config": "auto"}"#).unwrap();
        assert_eq!(o.islands_config, IslandConfigMode::Auto);
        for bad in [r#""on""#, "true", r#"".prettierrc""#] {
            let json = format!(r#"{{"islands.config": {bad}}}"#);
            assert!(Options::from_json(&json).is_err(), "{json}");
        }
    }

    #[test]
    fn islands_interpolated_values() {
        assert!(Options::default().islands_interpolated);
        for value in [true, false] {
            let json = format!(r#"{{"islands.interpolated": {value}}}"#);
            let (o, warnings) = Options::from_json(&json).unwrap();
            assert_eq!(o.islands_interpolated, value);
            assert!(warnings.is_empty());
        }
        let err = Options::from_json(r#"{"islands.interpolated": "off"}"#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("`islands.interpolated`"), "{err}");
    }

    #[test]
    fn tags_body_indent_values() {
        assert_eq!(Options::default().tags_body_indent, TagBodyIndent::Always);
        for (value, indent) in [
            ("always", TagBodyIndent::Always),
            ("cfml", TagBodyIndent::Cfml),
        ] {
            let json = format!(r#"{{"tags.body.indent": "{value}"}}"#);
            let (o, warnings) = Options::from_json(&json).unwrap();
            assert_eq!(o.tags_body_indent, indent);
            assert!(warnings.is_empty());
            assert_eq!(serde_json::to_value(indent).unwrap(), Value::from(value));
        }
        assert_eq!(
            Options::from_json(r#"{"tags.body.indent": "html"}"#)
                .unwrap_err()
                .to_string(),
            "invalid settings: `tags.body.indent`: unknown variant `html`, expected `always` or `cfml`"
        );
    }

    #[test]
    fn tags_islands_indent_values() {
        assert!(Options::default().tags_islands_indent);
        for value in [true, false] {
            let json = format!(r#"{{"tags.islands.indent": {value}}}"#);
            let (o, warnings) = Options::from_json(&json).unwrap();
            assert_eq!(o.tags_islands_indent, value);
            assert!(warnings.is_empty());
        }
        assert!(Options::from_json(r#"{"tags.islands.indent": "always"}"#).is_err());
    }

    #[test]
    fn reference_covers_every_key() {
        let Value::Object(defaults) = serde_json::to_value(Options::default()).unwrap() else {
            unreachable!()
        };
        let table: Vec<&str> = reference().iter().map(|i| i.key).collect();
        let mut sorted = table.clone();
        sorted.sort_unstable();
        assert_eq!(table, sorted, "the table is sorted by key");
        for key in defaults.keys() {
            assert!(
                table.contains(&key.as_str()),
                "`{key}` is not in the reference"
            );
        }
        for info in reference() {
            let default = defaults
                .get(info.key)
                .unwrap_or_else(|| panic!("`{}` is not an option", info.key));
            let literal: Value = serde_json::from_str(info.default).unwrap();
            assert_eq!(&literal, default, "default of `{}`", info.key);
            let domain: Vec<String> = match info.kind {
                Kind::Bool => vec!["true".into(), "false".into()],
                Kind::Enum(values) => values
                    .iter()
                    .map(|v| serde_json::to_string(v).unwrap())
                    .collect(),
                Kind::Separator => SEPARATOR_EXAMPLES.iter().map(|s| s.to_string()).collect(),
                Kind::Integer => vec![info.default.to_string()],
            };
            for value in domain {
                let json = format!(r#"{{"{}": {value}}}"#, info.key);
                assert!(Options::from_json(&json).is_ok(), "{json}");
            }
            if let Some(example) = info.example {
                if matches!(info.kind, Kind::Integer) {
                    assert_eq!(example.values.len(), 2, "`{}` shows two values", info.key);
                    assert_eq!(example.values[0], info.default);
                }
                assert!(matches!(
                    serde_json::from_str(example.settings),
                    Ok(Value::Object(_))
                ));
            }
        }
    }

    #[test]
    fn validation() {
        for ok in [":", ": ", " :", " : ", "=", " = ", "= "] {
            let json = format!(r#"{{"struct.separator": {ok:?}}}"#);
            assert!(Options::from_json(&json).is_ok(), "{ok:?}");
        }
        for bad in ["", "-", "::", "  :", ": =", " :  ", "a"] {
            let json = format!(r#"{{"struct.separator": {bad:?}}}"#);
            let err = Options::from_json(&json).unwrap_err().to_string();
            assert!(err.contains("struct.separator"), "{bad:?}: {err}");
        }
        for key in ["indent_size", "max_columns"] {
            let err = Options::from_json(&format!(r#"{{"{key}": 0}}"#))
                .unwrap_err()
                .to_string();
            assert!(err.contains(key), "{err}");
            assert!(Options::from_json(&format!(r#"{{"{key}": -1}}"#)).is_err());
        }
        assert!(Options::from_json(r#"{"struct.multiline.min_item_length": -1}"#).is_err());
    }

    /// A directory under the system temp dir, removed on drop.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cfformat-options-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn write(&self, rel: &str, contents: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, contents).unwrap();
            path
        }

        fn mkdir(&self, rel: &str) -> PathBuf {
            let path = self.0.join(rel);
            std::fs::create_dir_all(&path).unwrap();
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn discovery_walks_up_to_the_nearest_file() {
        let t = TempDir::new("walk");
        t.write("a/.cfformat.json", r#"{"indent_size": 2}"#);
        let near = t.write("a/b/.cfformat.json", r#"{"indent_size": 3}"#);
        t.mkdir("a/b/c");
        let d = Discovery::new(None, None);
        let r = d.discover(&t.0.join("a/b/c/File.cfc")).unwrap();
        assert_eq!(r.options.indent_size, 3);
        assert_eq!(r.sources, vec![near]);
        let r = d.discover(&t.0.join("a/File.cfc")).unwrap();
        assert_eq!(r.options.indent_size, 2);
        // The directory argument form.
        assert_eq!(
            d.discover_in(&t.0.join("a/b")).unwrap().options.indent_size,
            3
        );
        // A sibling tree does not see `a`'s files.
        let r = Discovery::new(None, None)
            .discover(&t.mkdir("elsewhere").join("x.cfc"))
            .unwrap();
        assert!(r.sources.iter().all(|s| !s.starts_with(&t.0)));
    }

    #[test]
    fn discovery_stops_at_a_git_directory() {
        let t = TempDir::new("git");
        t.write("a/.cfformat.json", r#"{"indent_size": 2}"#);
        t.mkdir("a/b/.git");
        t.mkdir("a/b/c");
        let d = Discovery::new(None, None);
        let r = d.discover(&t.0.join("a/b/c/File.cfc")).unwrap();
        assert!(r.sources.is_empty(), "{:?}", r.sources);
        assert_eq!(r.options, Options::default());
        // The settings file in the repository root itself is still found.
        let root = t.write("a/b/.cfformat.json", r#"{"indent_size": 5}"#);
        let r = Discovery::new(None, None)
            .discover(&t.0.join("a/b/c/File.cfc"))
            .unwrap();
        assert_eq!(r.sources, vec![root]);
        // A worktree's `.git` file stops the walk too.
        t.write("w/.cfformat.json", r#"{"indent_size": 2}"#);
        t.write("w/x/.git", "gitdir: elsewhere");
        let r = Discovery::new(None, None)
            .discover(&t.0.join("w/x/File.cfc"))
            .unwrap();
        assert!(r.sources.is_empty());
    }

    /// A home directory with a settings file that sets `tab_indent` and
    /// `max_columns`, and an explicit config setting `max_columns`.
    fn home_and_config(t: &TempDir) -> (PathBuf, PathBuf) {
        let home = t.write(
            "home/.cfformat.json",
            r#"{"indent_size": 2, "max_columns": 80, "tab_indent": true}"#,
        );
        let config = t.write("inline.json", r#"{"max_columns": 100}"#);
        (home, config)
    }

    #[test]
    fn discovery_takes_the_nearest_file_alone() {
        let t = TempDir::new("nearest-alone");
        let (home, config) = home_and_config(&t);
        let project = t.write("p/.cfformat.json", r#"{"indent_size": 3}"#);
        let r = Discovery::new(Some(home.clone()), None)
            .discover(&t.0.join("p/File.cfc"))
            .unwrap();
        assert_eq!(r.sources, vec![project.clone()]);
        assert_eq!(r.options.indent_size, 3);
        assert!(!r.options.tab_indent, "the home file is not merged");
        assert_eq!(r.options.max_columns, 120, "the home file is not merged");
        // `--config` over it.
        let r = Discovery::new(Some(home), Some(config.clone()))
            .discover(&t.0.join("p/File.cfc"))
            .unwrap();
        assert_eq!(r.sources, vec![project, config]);
        assert_eq!((r.options.indent_size, r.options.max_columns), (3, 100));
        assert!(!r.options.tab_indent);
    }

    #[test]
    fn discovery_falls_back_to_the_home_file() {
        let t = TempDir::new("home-fallback");
        let (home, config) = home_and_config(&t);
        t.mkdir("elsewhere/.git");
        let file = t.mkdir("elsewhere/src").join("File.cfc");
        let r = Discovery::new(Some(home.clone()), None)
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![home.clone()]);
        assert!(r.options.tab_indent);
        let r = Discovery::new(Some(home.clone()), Some(config.clone()))
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![home, config.clone()]);
        assert_eq!((r.options.max_columns, r.options.indent_size), (100, 2));
        // The fallback disabled, or missing: the defaults, then `--config`.
        let r = Discovery::new(None, None).discover(&file).unwrap();
        assert!(r.sources.is_empty());
        let r = Discovery::new(Some(t.0.join("nope/.cfformat.json")), Some(config.clone()))
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![config]);
        assert_eq!(r.options.max_columns, 100);
    }

    #[test]
    fn discovery_uses_the_home_file_past_a_git_directory() {
        // The walk stops at the repository inside the home directory, before
        // reaching the home file: it was not walked, so it is the fallback.
        let t = TempDir::new("home-git");
        let (home, config) = home_and_config(&t);
        t.mkdir("home/repo/.git");
        let file = t.mkdir("home/repo/src").join("File.cfc");
        let r = Discovery::new(Some(home.clone()), None)
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![home.clone()]);
        assert!(r.options.tab_indent);
        let r = Discovery::new(Some(home.clone()), Some(config.clone()))
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![home, config]);
    }

    #[test]
    fn discovery_reads_a_home_file_found_by_the_walk_once() {
        let t = TempDir::new("home-walk");
        let (home, config) = home_and_config(&t);
        let file = t.mkdir("home/notes").join("File.cfc");
        let d = Discovery::new(Some(home.clone()), None);
        let r = d.discover(&file).unwrap();
        assert_eq!(r.sources, vec![home.clone()]);
        assert_eq!(r.options.indent_size, 2);
        let r = d.discover(&t.0.join("home/File.cfc")).unwrap();
        assert_eq!(r.sources, vec![home.clone()]);
        let r = Discovery::new(Some(home.clone()), Some(config.clone()))
            .discover(&file)
            .unwrap();
        assert_eq!(r.sources, vec![home, config]);
    }

    #[test]
    fn discovery_errors_on_a_missing_config() {
        let t = TempDir::new("missing-config");
        let err = Discovery::new(None, Some(t.0.join("missing.json")))
            .discover(&t.0.join("p/File.cfc"))
            .unwrap_err();
        assert!(err.to_string().contains("missing.json"), "{err}");
    }

    #[test]
    fn discovery_reads_each_file_once_and_keeps_its_warnings() {
        let t = TempDir::new("warnings");
        let project = t.write(
            "p/.cfformat.json",
            r#"{"keywords.spacing_to_group": true, "function_call.padding": true}"#,
        );
        t.mkdir("p/q");
        let d = Discovery::new(None, None);
        let first = d.discover(&t.0.join("p/A.cfc")).unwrap();
        assert_eq!(first.warnings.len(), 2);
        assert!(first.warnings.iter().all(|(p, _)| *p == project));
        assert!(first.options.parentheses_padding);
        // Rewriting the file does not change the answer: it was read once.
        std::fs::write(&project, r#"{"indent_size": 7}"#).unwrap();
        let second = d.discover(&t.0.join("p/q/B.cfc")).unwrap();
        assert_eq!(second.warnings, first.warnings);
        assert_eq!(second.options, first.options);
        // A file that fails fails once: fixing it mid-run changes nothing.
        let bad = t.write("r/.cfformat.json", r#"{"indent_size": "x"}"#);
        t.mkdir("r/s");
        let error = d.discover(&t.0.join("r/A.cfc")).unwrap_err();
        assert!(error.to_string().contains("indent_size"), "{error}");
        std::fs::write(&bad, "{}").unwrap();
        assert_eq!(d.discover(&t.0.join("r/s/B.cfc")).unwrap_err(), error);
        assert_eq!(d.reads.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn discovery_reads_a_file_once_whatever_the_threads() {
        let t = TempDir::new("threads");
        t.write("p/.cfformat.json", r#"{"indent_size": 2}"#);
        let d = Discovery::new(None, None);
        let file = t.0.join("p/A.cfc");
        let resolved: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| d.discover(&file).unwrap().options.indent_size))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(resolved, [2; 8]);
        assert_eq!(d.reads.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(d.files.lock().unwrap().len(), 1);
    }

    #[test]
    fn discovery_errors_name_the_file() {
        let t = TempDir::new("errors");
        for (dir, json, what) in [
            ("a", "{", "EOF"),
            ("b", "[]", "object"),
            ("c", r#"{"max_colums": 1}"#, "max_colums"),
            ("d", r#"{"struct.separator": "->"}"#, "struct.separator"),
        ] {
            t.write(&format!("{dir}/.cfformat.json"), json);
            // The path as discovery joins it (a backslash on Windows).
            let path = t.0.join(dir).join(SETTINGS_FILE);
            let err = Discovery::new(None, None)
                .discover(&t.0.join(dir).join("File.cfc"))
                .unwrap_err();
            let text = err.to_string();
            assert!(
                text.contains(&*path.to_string_lossy()) && text.contains(what),
                "{text}"
            );
        }
    }

    #[test]
    fn newline_values() {
        let (o, _) = Options::from_json(r#"{"newline": "\r\n"}"#).unwrap();
        assert_eq!(o.newline_str(cfparse::Newline::Lf), "\r\n");
        let (o, _) = Options::from_json(r#"{"newline": "auto"}"#).unwrap();
        assert_eq!(o.newline_str(cfparse::Newline::CrLf), "\r\n");
        let (o, _) = Options::from_json(r#"{"newline": "\n"}"#).unwrap();
        assert_eq!(o.newline_str(cfparse::Newline::CrLf), "\n");
    }
}
