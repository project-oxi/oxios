//! Format-aware note writing on the shared frontmatter contract.
//!
//! Vault-unification T12: every note write funnels through the
//! `oxi-frontmatter` contract crate (constrained YAML, unknown-key
//! preserving, atomic tmp+fsync+rename) instead of hand-rolled serde_yaml
//! emission. This module is the oxios-markdown-side facade over the
//! contract crate; `knowledge.rs` migrates its write sites onto
//! [`write_note`] incrementally.
//!
//! Status: facade wired and compiling; the knowledge-base write-site
//! conversion lands with the rest of the vault-unification tasks.

pub use oxi_frontmatter::{
    FrontmatterError, Mutation, NoteFormat, Parsed, Synthesize, Table, Value, WriteOutcome,
    atomic_write, emit, parse, write_document,
};

use std::path::Path;

/// Merge-write a note body + canonical frontmatter block to `path`.
///
/// Thin convenience over [`write_document`] for the common note-write
/// case: carry the existing table forward (preserving unknown keys),
/// apply `mutations`, refresh the core identity invariants
/// (`id`/`created`/`updated`), and atomically replace the file.
pub fn write_note(
    path: &Path,
    body: &str,
    fmt: NoteFormat,
    mutations: Mutation,
    synth: Synthesize,
    now: time::OffsetDateTime,
) -> Result<WriteOutcome, FrontmatterError> {
    write_document(path, body, fmt, mutations, synth, now)
}
