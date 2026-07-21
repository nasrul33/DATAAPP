use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DurationClass {
    Short,
    Medium,
    Long,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceEstimate {
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub duration: DurationClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceBudget {
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub max_duration: DurationClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceError {
    InvalidBudget,
    MemoryExceeded,
    DiskExceeded,
    DurationExceeded,
    ArithmeticOverflow,
    Poisoned,
}

#[derive(Debug, Default)]
struct Reserved {
    memory_bytes: u64,
    disk_bytes: u64,
}

#[derive(Debug)]
struct LedgerInner {
    budget: ResourceBudget,
    reserved: Mutex<Reserved>,
}

#[derive(Debug, Clone)]
pub struct ResourceLedger {
    inner: Arc<LedgerInner>,
}

#[derive(Debug)]
pub struct ResourceReservation {
    inner: Arc<LedgerInner>,
    estimate: ResourceEstimate,
}

impl ResourceLedger {
    /// Create a ledger with a non-zero memory and disk budget.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceError::InvalidBudget`] when either byte budget is zero.
    pub fn new(budget: ResourceBudget) -> Result<Self, ResourceError> {
        if budget.memory_bytes == 0 || budget.disk_bytes == 0 {
            return Err(ResourceError::InvalidBudget);
        }

        Ok(Self {
            inner: Arc::new(LedgerInner {
                budget,
                reserved: Mutex::new(Reserved::default()),
            }),
        })
    }

    /// Reserve resources until the returned guard is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when the estimate exceeds the duration or aggregate
    /// resource budget, arithmetic overflows, or the ledger mutex is poisoned.
    pub fn reserve(
        &self,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        if estimate.duration > self.inner.budget.max_duration {
            return Err(ResourceError::DurationExceeded);
        }

        let mut reserved = self
            .inner
            .reserved
            .lock()
            .map_err(|_| ResourceError::Poisoned)?;
        let memory_bytes = reserved
            .memory_bytes
            .checked_add(estimate.memory_bytes)
            .ok_or(ResourceError::ArithmeticOverflow)?;
        let disk_bytes = reserved
            .disk_bytes
            .checked_add(estimate.disk_bytes)
            .ok_or(ResourceError::ArithmeticOverflow)?;

        if memory_bytes > self.inner.budget.memory_bytes {
            return Err(ResourceError::MemoryExceeded);
        }
        if disk_bytes > self.inner.budget.disk_bytes {
            return Err(ResourceError::DiskExceeded);
        }

        reserved.memory_bytes = memory_bytes;
        reserved.disk_bytes = disk_bytes;

        Ok(ResourceReservation {
            inner: Arc::clone(&self.inner),
            estimate,
        })
    }

    #[cfg(test)]
    fn reserved_for_test(&self) -> (u64, u64) {
        match self.inner.reserved.lock() {
            Ok(reserved) => (reserved.memory_bytes, reserved.disk_bytes),
            Err(poisoned) => {
                let reserved = poisoned.into_inner();
                (reserved.memory_bytes, reserved.disk_bytes)
            }
        }
    }

    #[cfg(test)]
    pub(super) fn poison_for_test(&self) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _reserved = self.inner.reserved.lock().expect("unpoisoned ledger");
            panic!("poison resource ledger for test");
        }));
        assert!(result.is_err());
    }
}

impl Drop for ResourceReservation {
    fn drop(&mut self) {
        let mut reserved = match self.inner.reserved.lock() {
            Ok(reserved) => reserved,
            Err(poisoned) => poisoned.into_inner(),
        };
        let memory_bytes = reserved
            .memory_bytes
            .checked_sub(self.estimate.memory_bytes);
        let disk_bytes = reserved.disk_bytes.checked_sub(self.estimate.disk_bytes);
        let (Some(memory_bytes), Some(disk_bytes)) = (memory_bytes, disk_bytes) else {
            panic!("resource ledger reservation invariant violated");
        };
        reserved.memory_bytes = memory_bytes;
        reserved.disk_bytes = disk_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::{DurationClass, ResourceBudget, ResourceError, ResourceEstimate, ResourceLedger};

    fn estimate(memory_bytes: u64, disk_bytes: u64, duration: DurationClass) -> ResourceEstimate {
        ResourceEstimate {
            memory_bytes,
            disk_bytes,
            duration,
        }
    }

    #[test]
    fn reservations_are_checked_aggregated_and_released() {
        let ledger = ResourceLedger::new(ResourceBudget {
            memory_bytes: 100,
            disk_bytes: 200,
            max_duration: DurationClass::Medium,
        })
        .expect("valid budget");
        let first = ledger
            .reserve(estimate(60, 80, DurationClass::Short))
            .expect("first reservation");
        assert!(matches!(
            ledger.reserve(estimate(50, 10, DurationClass::Short)),
            Err(ResourceError::MemoryExceeded)
        ));
        drop(first);
        assert_eq!(ledger.reserved_for_test(), (0, 0));
        assert!(ledger
            .reserve(estimate(100, 200, DurationClass::Medium))
            .is_ok());
    }

    #[test]
    fn corrupted_reservation_accounting_fails_closed_on_drop() {
        let ledger = ResourceLedger::new(ResourceBudget {
            memory_bytes: 10,
            disk_bytes: 10,
            max_duration: DurationClass::Short,
        })
        .expect("valid budget");
        let reservation = ledger
            .reserve(estimate(1, 1, DurationClass::Short))
            .expect("reservation");
        {
            let mut reserved = ledger.inner.reserved.lock().expect("unpoisoned ledger");
            reserved.memory_bytes = 0;
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            drop(reservation);
        }));

        assert!(result.is_err());
    }

    #[test]
    fn invalid_or_overlong_work_is_rejected_without_reservation() {
        assert!(matches!(
            ResourceLedger::new(ResourceBudget {
                memory_bytes: 0,
                disk_bytes: 1,
                max_duration: DurationClass::Long,
            }),
            Err(ResourceError::InvalidBudget)
        ));
        let ledger = ResourceLedger::new(ResourceBudget {
            memory_bytes: 10,
            disk_bytes: 10,
            max_duration: DurationClass::Short,
        })
        .expect("valid budget");
        assert!(matches!(
            ledger.reserve(estimate(1, 1, DurationClass::Long)),
            Err(ResourceError::DurationExceeded)
        ));
        assert_eq!(ledger.reserved_for_test(), (0, 0));
    }
}
