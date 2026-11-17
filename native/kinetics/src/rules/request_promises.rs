//! Request promises (`RequestPromise`, `RequestPromiseQueue`): what a Factory Gauge or a Stock
//! Ticker order has asked the network for and that hasn't arrived yet. A promise counts against
//! the stock a gauge is waiting for until the items turn up (`itemEnteredSystem`) or it expires
//! (`getTotalPromisedAndRemoveExpired`).
//!
//! No game types: `K` is an item with its components, equal when
//! `ItemStack.isSameItemSameComponents`.
//!
//! Known differences from Create, for the README:
//! - Create keeps a queue's promises in an identity hash map by `Item`, each with a list;
//!   [`Queue`] keeps one list in the order they were added, and the matching is by the full key
//!   `K`. The results are the same. [`Queue::is_empty`] is true when there is no promise;
//!   Create's map can keep an item with an empty list after a promise expires.
//! - `onChanged` (marking the world data or the block entity dirty) is a flag the caller reads
//!   with [`Queue::take_changed`].

use super::package::Big;

/// `RequestPromise`: a promised stack and the ticks it has existed (`ticksExisted`).
#[derive(Clone, Debug, PartialEq)]
pub struct Promise<K> {
    pub ticks_existed: i32,
    pub stack: Big<K>,
}

impl<K> Promise<K> {
    /// `new RequestPromise(promisedStack)`.
    pub fn new(stack: Big<K>) -> Self {
        Self { ticks_existed: 0, stack }
    }
}

/// `RequestPromiseQueue`.
#[derive(Clone, Debug, PartialEq)]
pub struct Queue<K> {
    promises: Vec<Promise<K>>,
    changed: bool,
}

impl<K> Default for Queue<K> {
    fn default() -> Self {
        Self { promises: Vec::new(), changed: false }
    }
}

impl<K: PartialEq + Clone> Queue<K> {
    /// `RequestPromiseQueue.add`.
    pub fn add(&mut self, promise: Promise<K>) {
        self.promises.push(promise);
        self.changed = true;
    }

    /// Whether `onChanged` ran since the last call (and clears it).
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// `RequestPromiseQueue.getTotalPromisedAndRemoveExpired`: the count promised of `key`; with
    /// an `expiry_time` other than -1, a promise that has existed that many ticks is removed
    /// instead of counted.
    pub fn total_promised_and_remove_expired(&mut self, key: &K, expiry_time: i32) -> u32 {
        let mut promised: u32 = 0;
        let mut changed = false;
        self.promises.retain(|promise| {
            if promise.stack.0 != *key {
                return true;
            }
            if expiry_time != -1 && promise.ticks_existed >= expiry_time {
                changed = true;
                return false;
            }
            promised = promised.saturating_add(promise.stack.1);
            true
        });
        self.changed |= changed;
        promised
    }

    /// `RequestPromiseQueue.forceClear`: every promise of `key` is dropped.
    pub fn force_clear(&mut self, key: &K) {
        let before = self.promises.len();
        self.promises.retain(|promise| promise.stack.0 != *key);
        if self.promises.len() != before {
            self.changed = true;
        }
    }

    /// `RequestPromiseQueue.itemEnteredSystem`: `amount` of `key` arrived; the oldest promises of
    /// it are reduced (and removed once nothing is left of them) until it is used up.
    pub fn item_entered_system(&mut self, key: &K, mut amount: u32) {
        let mut changed = false;
        let mut done = false;
        self.promises.retain_mut(|promise| {
            if done || promise.stack.0 != *key {
                return true;
            }
            let subtract = amount.min(promise.stack.1);
            amount -= subtract;
            promise.stack.1 -= subtract;
            let keep = promise.stack.1 > 0;
            if !keep {
                changed = true;
            }
            if amount == 0 {
                done = true;
            }
            keep
        });
        self.changed |= changed;
    }

    /// `RequestPromiseQueue.flatten(sorted)`: every promise; sorted, the oldest first
    /// (`RequestPromise.ageComparator`).
    pub fn flatten(&self, sorted: bool) -> Vec<Promise<K>> {
        let mut all = self.promises.clone();
        if sorted {
            all.sort_by_key(|p| std::cmp::Reverse(p.ticks_existed));
        }
        all
    }

    /// `RequestPromiseQueue.tick`: every promise a tick older.
    pub fn tick(&mut self) {
        for promise in &mut self.promises {
            promise.ticks_existed += 1;
        }
    }

    /// `RequestPromiseQueue.isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.promises.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn promise(key: &'static str, count: u32) -> Promise<&'static str> {
        Promise::new((key, count))
    }

    #[test]
    fn a_promise_counts_until_it_expires() {
        let mut queue = Queue::default();
        queue.add(promise("gear", 4));
        queue.add(promise("gear", 2));
        queue.add(promise("shaft", 9));
        assert!(queue.take_changed());
        assert!(!queue.take_changed());
        // Never expiring (-1): both gear promises count.
        assert_eq!(queue.total_promised_and_remove_expired(&"gear", -1), 6);
        // A gauge clearing at interval 0 expires promises after 20 * 30 = 600 ticks
        // (`getPromiseExpiryTimeInTicks`): at 599 ticks they still count.
        for _ in 0..599 {
            queue.tick();
        }
        assert_eq!(queue.total_promised_and_remove_expired(&"gear", 600), 6);
        // The 600th tick: `ticksExisted >= expiryTime` removes them, and only the gears.
        queue.tick();
        assert_eq!(queue.total_promised_and_remove_expired(&"gear", 600), 0);
        assert!(queue.take_changed());
        // Another item's promise isn't touched by asking about gears.
        assert_eq!(queue.flatten(false), vec![Promise { ticks_existed: 600, stack: ("shaft", 9) }]);
    }

    #[test]
    fn items_entering_use_up_the_oldest_promises_first() {
        let mut queue = Queue::default();
        queue.add(promise("gear", 4));
        for _ in 0..5 {
            queue.tick();
        }
        queue.add(promise("gear", 3));
        queue.take_changed();
        // Five arrive: the first promise (4) is used up and removed, the second drops by one.
        queue.item_entered_system(&"gear", 5);
        assert_eq!(queue.flatten(false), vec![Promise { ticks_existed: 0, stack: ("gear", 2) }]);
        assert!(queue.take_changed());
        // More than promised: the queue empties and the rest is lost.
        queue.item_entered_system(&"gear", 50);
        assert!(queue.is_empty());
        // Nothing promised of that item: nothing happens, and nothing is flagged.
        queue.take_changed();
        queue.item_entered_system(&"cog", 3);
        assert!(!queue.take_changed());
    }

    #[test]
    fn force_clear_and_flatten() {
        let mut queue = Queue::default();
        queue.add(promise("gear", 1));
        queue.tick();
        queue.add(promise("shaft", 2));
        queue.add(promise("gear", 3));
        // Sorted: the oldest first (the 1-tick-old gear, then the two new ones in order).
        let sorted = queue.flatten(true);
        assert_eq!(sorted[0].stack, ("gear", 1));
        assert_eq!(sorted.len(), 3);
        queue.take_changed();
        queue.force_clear(&"gear");
        assert_eq!(queue.flatten(false), vec![promise("shaft", 2)]);
        assert!(queue.take_changed());
        queue.force_clear(&"gear");
        assert!(!queue.take_changed());
    }
}
