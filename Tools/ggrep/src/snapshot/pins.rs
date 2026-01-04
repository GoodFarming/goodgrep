//! Snapshot pin tracking for daemon queries.

use std::collections::{HashMap, HashSet};

#[cfg(feature = "loom")]
use loom::sync::Mutex;
#[cfg(not(feature = "loom"))]
use parking_lot::Mutex;

#[derive(Default)]
pub struct SnapshotPins {
   inner: Mutex<HashMap<String, usize>>,
}

impl SnapshotPins {
   pub fn pin(&self, snapshot_id: &str) {
      let mut pins = self.inner.lock();
      let counter = pins.entry(snapshot_id.to_string()).or_insert(0);
      *counter = counter.saturating_add(1);
   }

   pub fn unpin(&self, snapshot_id: &str) {
      let mut pins = self.inner.lock();
      if let Some(count) = pins.get_mut(snapshot_id) {
         if *count > 1 {
            *count -= 1;
         } else {
            pins.remove(snapshot_id);
         }
      }
   }

   pub fn ids(&self) -> HashSet<String> {
      let pins = self.inner.lock();
      pins.keys().cloned().collect()
   }
}

#[cfg(all(test, feature = "loom"))]
mod tests {
   use loom::{sync::Arc, thread};

   use super::SnapshotPins;

   #[test]
   fn pins_are_balanced_under_concurrency() {
      loom::model(|| {
         let pins = Arc::new(SnapshotPins::default());
         let first = Arc::clone(&pins);
         let second = Arc::clone(&pins);

         let t1 = thread::spawn(move || {
            first.pin("snap");
            first.unpin("snap");
         });
         let t2 = thread::spawn(move || {
            second.pin("snap");
            second.unpin("snap");
         });

         t1.join().expect("thread 1");
         t2.join().expect("thread 2");

         assert!(pins.ids().is_empty());
      });
   }
}
