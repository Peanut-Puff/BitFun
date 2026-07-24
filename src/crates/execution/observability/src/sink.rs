use crate::ValidatedRecord;
use std::sync::Mutex;

/// Non-blocking destination for records that already passed the privacy gate.
///
/// Implementations must isolate failures from product control flow. A network
/// implementation should enqueue with a bounded `try_send` and account for
/// drops internally instead of blocking this method.
pub trait TelemetrySink: Send + Sync + 'static {
    fn emit(&self, record: ValidatedRecord);

    fn discard_pending(&self) {}
}

#[derive(Debug, Default)]
pub struct NoopSink;

impl TelemetrySink for NoopSink {
    fn emit(&self, _record: ValidatedRecord) {}
}

#[derive(Debug, Default)]
pub struct InMemorySink {
    records: Mutex<Vec<ValidatedRecord>>,
}

impl InMemorySink {
    pub fn records(&self) -> Vec<ValidatedRecord> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn take(&self) -> Vec<ValidatedRecord> {
        std::mem::take(
            &mut *self
                .records
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
    }
}

impl TelemetrySink for InMemorySink {
    fn emit(&self, record: ValidatedRecord) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(record);
    }

    fn discard_pending(&self) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}
