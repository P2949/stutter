mod collision;
mod decay;
mod diagnostics;
pub(crate) mod key;
mod model;
mod persistence;

pub(crate) use key::CandidateContextHashInput;
pub(crate) use model::{
    CandidateMemory, CandidateMemoryRecord, CandidateMemoryResult, CandidateResultRecordInput,
};

#[cfg(test)]
mod tests;
