use anyhow::{format_err, Error};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct FailureCount {
    max_count: usize,
    counter: AtomicUsize,
}

impl FailureCount {
    #[must_use]
    pub fn new(max_count: usize) -> Self {
        Self {
            max_count,
            counter: AtomicUsize::new(0),
        }
    }

    /// # Errors
    /// Returns error if more than `max_count` errors are encountered
    pub fn check(&self) -> Result<(), Error> {
        if self.counter.load(Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }

    /// # Errors
    /// Returns error if more than `max_count` errors are encountered
    pub fn reset(&self) -> Result<(), Error> {
        if self.counter.swap(0, Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }

    /// # Errors
    /// Returns error if more than `max_count` errors are encountered
    pub fn increment(&self) -> Result<(), Error> {
        if self.counter.fetch_add(1, Ordering::SeqCst) > self.max_count {
            Err(format_err!(
                "Failed after retrying {} times",
                self.max_count
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Error;

    use crate::failure_count::FailureCount;

    #[test]
    fn test_failure_count() -> Result<(), Error> {
        let count = FailureCount::new(1);
        assert!(count.check().is_ok());
        assert!(count.increment().is_ok());
        assert!(count.increment().is_ok());
        assert!(count.increment().is_err());
        assert!(count.reset().is_err());
        assert!(count.check().is_ok());
        Ok(())
    }
}
