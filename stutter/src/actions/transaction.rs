use crate::actions::{PartialApplyError, RollbackToken};

pub struct ApplyTransaction<R> {
    pub applied: Vec<R>,
}

impl<R> ApplyTransaction<R> {
    pub fn new() -> Self {
        Self {
            applied: Vec::new(),
        }
    }

    pub fn apply_loop<I, F, T>(
        mut self,
        items: I,
        mut apply_fn: F,
        make_token: T,
    ) -> Result<RollbackToken, PartialApplyError>
    where
        I: IntoIterator,
        F: FnMut(I::Item) -> Result<R, anyhow::Error>,
        T: FnOnce(Vec<R>) -> RollbackToken,
    {
        for item in items {
            match apply_fn(item) {
                Ok(record) => {
                    self.applied.push(record);
                }
                Err(err) => {
                    let rollback = if self.applied.is_empty() {
                        None
                    } else {
                        Some(make_token(self.applied))
                    };
                    return Err(PartialApplyError {
                        source: err,
                        rollback,
                    });
                }
            }
        }
        Ok(make_token(self.applied))
    }
}
