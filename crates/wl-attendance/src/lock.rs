//! One lock per employee and day. Two supervisors who mark the same worker
//! at the same moment must not both check for an overlap before either has
//! saved, so the check and the insert of one employee and day run one at a
//! time (wl-attendance §4 *record*, BL-ATT-01 edge cases).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use renox::chrono::NaiveDate;
use renox::tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

type Locks = Mutex<HashMap<(i64, NaiveDate), Arc<AsyncMutex<()>>>>;

fn locks() -> &'static Locks {
    static LOCKS: OnceLock<Locks> = OnceLock::new();
    LOCKS.get_or_init(Mutex::default)
}

/// The lock of one employee and day, released when dropped.
#[derive(Debug)]
pub struct DayLock(#[allow(dead_code)] OwnedMutexGuard<()>);

/// Takes the lock of `employee` on `date`, waiting for any save that holds
/// it. Locks nobody holds or waits for are dropped from the table on the way.
pub async fn take(employee: i64, date: NaiveDate) -> DayLock {
    let mutex = {
        let mut table = locks().lock().unwrap_or_else(|e| e.into_inner());
        table.retain(|_, m| Arc::strong_count(m) > 1);
        Arc::clone(table.entry((employee, date)).or_default())
    };
    DayLock(mutex.lock_owned().await)
}
