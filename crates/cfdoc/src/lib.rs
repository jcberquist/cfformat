//! A Rust port of Prettier's document printer (`prettier/src/document`).
//!
//! Build a [`Doc`] with the [`builders`], then lay it out with [`print_doc`]:
//! every [`group`](builders::group) is printed flat if it fits in the
//! remaining width, otherwise its lines break. The crate knows nothing about
//! the language being formatted.
//!
//! ```
//! use cfdoc::builders::{group, indent, join, line, softline};
//! use cfdoc::{print_doc, Doc, IndentStyle, PrintOptions};
//!
//! let items = ["alpha", "beta", "gamma"].map(Doc::from);
//! let doc = group(vec![
//!     "[".into(),
//!     indent(vec![softline(), join(vec![",".into(), line()], items)]),
//!     softline(),
//!     "]".into(),
//! ]);
//!
//! let wide = PrintOptions { width: 80, indent: IndentStyle::Spaces(4), newline: "\n" };
//! assert_eq!(print_doc(&mut doc.clone(), &wide), "[alpha, beta, gamma]");
//!
//! let narrow = PrintOptions { width: 16, ..wide };
//! assert_eq!(
//!     print_doc(&mut doc.clone(), &narrow),
//!     "[\n    alpha,\n    beta,\n    gamma\n]"
//! );
//! ```
//!
//! Two additions are not in Prettier:
//! [`builders::conditional_group_contents`], a one-state conditional group
//! stored without a copy, and [`utils::flat_width`], the width a doc takes on
//! one line, for a formatter choosing between layouts before printing.
//!
//! Not ported: the `cursor`, `trim` and `label` docs. See `README.md`.

pub mod builders;
pub mod debug;
mod doc;
pub mod indent;
mod printer;
pub mod utils;
pub mod width;

pub use doc::{Align, Doc, Group, GroupId, GroupIdGen, LineKind};
pub use indent::IndentStyle;
pub use printer::{print_doc, PrintOptions};
pub use utils::FlatWidth;
