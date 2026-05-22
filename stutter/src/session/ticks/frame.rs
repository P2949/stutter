#![allow(dead_code)] // Transitional frame tick context.
// Exit: remove this marker once the described migration is complete and the local allow is no longer needed.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrameTick;
