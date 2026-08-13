//! Runtime memory glue retained in the kernel after the oxibrain migration
//! (RFC-047).
//!
//! The oxibrain daemon owns extraction, classification, and salience-based
//! decay. The kernel keeps only [`sona`] — the SONA trajectory pattern engine
//! (runtime concern: distilling successful execution patterns from agent
//! trajectories). Everything else that was in `oxios-memory` is gone.

pub mod sona;
