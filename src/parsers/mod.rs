//! Parsers for different job definition formats.
//!
//! This module contains parsers for various formats that can be used
//! to define jobs in Petit.

pub mod poml;

pub use poml::PomlParser;
