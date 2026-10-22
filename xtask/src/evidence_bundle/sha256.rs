//! SHA-256 lives in `stutter-core` so the validation runner and the verifier share exactly one
//! implementation; this module only keeps the historical `evidence_bundle::sha256` path.
pub(crate) use stutter_core::hash::sha256_hex;
