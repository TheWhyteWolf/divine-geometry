//! The construction engine: registries, intersection solving, and the scripting
//! API figures are written against.
//!
//! Parts of this surface are deliberately broader than the current figure set
//! uses — `line_ext`, `arc_major` and the registry accessors exist because a
//! construction language with holes in it pushes the missing cases back into the
//! figure scripts, which is exactly what this crate is trying to avoid.
#![allow(dead_code)]

pub mod build;
pub mod construction;
pub mod isect;
pub mod registry;
