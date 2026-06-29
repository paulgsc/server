
/// Lock-free Single-Producer Single-Consumer ring buffer.
///
/// # Invariants
///
/// - Exactly one thread calls `try_push` (the producer).
/// - Exactly one thread calls `try_pop` (the consumer).
/// - Violating either invariant is undefined behavior.
///
/// # Memory ordering
///
/// - Producer: Relaxed load of `tail`, Acquire load of `head`, Release store of `tail`.
/// - Consumer: Acquire load of `tail`, Relaxed load of `head`, Release store of `head`.
///
/// The Acquire/Release pairs establish the happens-before edge:
/// - The write into `buffer[tail]` happens-before the consumer reads it.
/// - The slot becoming available happens-before the producer writes to it again.
///
/// No SeqCst is needed for a two-party protocol. See `docs/adr/003-spsc-ring-buffer.md`.
///
/// # Capacity
///
/// `N` must be a power of two. This allows index masking (`& (N-1)`) instead
/// of modulo, which compiles to a single AND instruction.
///
/// Effective capacity is `N - 1` slots (one slot is always kept empty to
/// distinguish full from empty without a separate counter).
use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct SpscRingBuffer<T, const N: usize> {
    buffer: [UnsafeCell<MaybeUninit<T>>; N],
    /// Written by producer (try_push), read by consumer (try_pop).
    tail: AtomicUsize,
    /// Written by consumer (try_pop), read by producer (try_push).
    head: AtomicUsize,
}

// Safety: The atomic index protocol ensures that only one thread accesses any
// given slot at a time. The UnsafeCell interior mutability is required to allow
// writes from the producer and reads from the consumer without &mut aliasing.
// T: Send is required because T crosses thread boundaries.
unsafe impl<T: Send, const N: usize> Send for SpscRingBuffer<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for SpscRingBuffer<T, N> {}

impl<T, const N: usize> SpscRingBuffer<T, N> {
    /// Assert at compile time that N is a power of two.
    ///
    /// This produces a compile error if N is not a power of two, preventing
    /// silent correctness bugs from the masking arithmetic.
    const POWER_OF_TWO_CHECK: () = {
        assert!(N.is_power_of_two(), "SpscRingBuffer: N must be a power of two");
    };

    const MASK: usize = N - 1;

    /// Create a new, empty ring buffer.
    ///
    /// Call this once at startup. The buffer is heap-allocated (via Box or Arc)
    /// so it can be shared across threads.
    ///
    /// # Panics
    ///
    /// Panics at compile time if N is not a power of two.
    #[allow(clippy::let_unit_value)]
    pub fn new() -> Self {
        // Trigger the compile-time check
        let _ = Self::POWER_OF_TWO_CHECK;

        // Safety: MaybeUninit arrays can be initialized this way.
        // The atomic indices ensure no uninit slot is ever read.
        let buffer = std::array::from_fn(|_| UnsafeCell::new(MaybeUninit::uninit()));

        Self {
            buffer,
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
        }
    }

    /// Attempt to push an item. Non-blocking.
    ///
    /// # Returns
    ///
    /// - `Ok(())` if the item was enqueued.
    /// - `Err(item)` if the buffer is full. The item is returned to the caller.
    ///
    /// # Real-time guarantees
    ///
    /// - Zero allocations.
    /// - Non-blocking (no park, no sleep, no mutex).
    /// - O(1) time.
    #[inline(always)]
    pub fn try_push(&self, item: T) -> Result<(), T> {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);

        let next_tail = (tail + 1) & Self::MASK;
        if next_tail == head {
            // Buffer full — return item immediately
            return Err(item);
        }

        // Safety: tail is owned by the producer. head has been Acquire-loaded,
        // establishing that the consumer has finished reading any slot up to head.
        // next_tail != head, so this slot is not being read by the consumer.
        unsafe {
            (*self.buffer[tail].get()).write(item);
        }

        // Release: makes the write above visible to the consumer before it sees
        // the updated tail.
        self.tail.store(next_tail, Ordering::Release);
        Ok(())
    }

    /// Attempt to pop an item. Non-blocking.
    ///
    /// # Returns
    ///
    /// - `Some(item)` if an item was available.
    /// - `None` if the buffer is empty.
    ///
    /// # Real-time guarantees
    ///
    /// - Zero allocations.
    /// - Non-blocking (no park, no sleep, no mutex).
    /// - O(1) time.
    #[inline(always)]
    pub fn try_pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);

        if head == tail {
            // Buffer empty
            return None;
        }

        // Safety: head is owned by the consumer. tail has been Acquire-loaded,
        // establishing that the producer's write to buffer[head] happened-before
        // this read.
        let item = unsafe { (*self.buffer[head].get()).assume_init_read() };

        // Release: makes the slot available to the producer.
        self.head.store((head + 1) & Self::MASK, Ordering::Release);
        Some(item)
    }

    /// True if the buffer contains no items.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire) == self.tail.load(Ordering::Acquire)
    }

    /// Approximate number of items currently in the buffer.
    ///
    /// This is approximate because head and tail are read separately
    /// without synchronization between the two loads.
    #[inline(always)]
    pub fn len_approx(&self) -> usize {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Relaxed);
        tail.wrapping_sub(head) & Self::MASK
    }

    /// Maximum number of items the buffer can hold.
    pub const fn capacity() -> usize {
        N - 1
    }
}

impl<T, const N: usize> Drop for SpscRingBuffer<T, N> {
    fn drop(&mut self) {
        // Drain any remaining items to run their destructors
        while self.try_pop().is_some() {}
    }
}

// MILESTONE M3 TESTS
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn push_pop_roundtrip() {
        let buf = SpscRingBuffer::<u32, 8>::new();
        assert!(buf.try_push(42).is_ok());
        assert_eq!(buf.try_pop(), Some(42));
    }

    #[test]
    fn empty_pop_returns_none() {
        let buf = SpscRingBuffer::<u32, 8>::new();
        assert_eq!(buf.try_pop(), None);
    }

    #[test]
    fn full_push_returns_err_immediately() {
        let buf = SpscRingBuffer::<u32, 4>::new(); // effective capacity = 3
        assert!(buf.try_push(1).is_ok());
        assert!(buf.try_push(2).is_ok());
        assert!(buf.try_push(3).is_ok());
        let result = buf.try_push(4); // should be full
        assert!(result.is_err(), "full buffer must return Err immediately, not block");
        // The returned item must be the one we tried to push
        assert_eq!(result.unwrap_err(), 4);
    }

    #[test]
    fn fifo_ordering() {
        let buf = SpscRingBuffer::<u32, 8>::new();
        for i in 0..5_u32 {
            buf.try_push(i).unwrap();
        }
        for i in 0..5_u32 {
            assert_eq!(buf.try_pop(), Some(i));
        }
    }

    #[test]
    fn wrap_around() {
        // Fill, drain, fill again — verifies index wrapping
        let buf = SpscRingBuffer::<u32, 4>::new(); // capacity = 3
        buf.try_push(1).unwrap();
        buf.try_push(2).unwrap();
        buf.try_push(3).unwrap();
        assert_eq!(buf.try_pop(), Some(1));
        assert_eq!(buf.try_pop(), Some(2));
        // Now push 2 more — indices have wrapped
        buf.try_push(4).unwrap();
        buf.try_push(5).unwrap();
        assert_eq!(buf.try_pop(), Some(3));
        assert_eq!(buf.try_pop(), Some(4));
        assert_eq!(buf.try_pop(), Some(5));
        assert_eq!(buf.try_pop(), None);
    }

    #[test]
    fn threaded_producer_consumer() {
        // One producer thread, one consumer thread. 10_000 items.
        // This is the critical correctness test for the atomic ordering.
        let buf = Arc::new(SpscRingBuffer::<u64, 1024>::new());
        let buf_producer = Arc::clone(&buf);
        let buf_consumer = Arc::clone(&buf);

        const ITEMS: u64 = 10_000;

        let producer = std::thread::spawn(move || {
            let mut sent = 0_u64;
            while sent < ITEMS {
                if buf_producer.try_push(sent).is_ok() {
                    sent += 1;
                }
                // spin - in real code you'd have a short yield here
            }
        });

        let consumer = std::thread::spawn(move || {
            let mut received = 0_u64;
            let mut last = u64::MAX;
            while received < ITEMS {
                if let Some(item) = buf_consumer.try_pop() {
                    // Verify monotonic ordering
                    if last != u64::MAX {
                        assert_eq!(item, last + 1, "FIFO ordering violated");
                    }
                    last = item;
                    received += 1;
                }
            }
        });

        producer.join().unwrap();
        consumer.join().unwrap();
    }

    // Compile-time test: this must NOT compile if N is not a power of two.
    // Uncomment to verify:
    // #[test]
    // fn non_power_of_two_fails_to_compile() {
    //     let _ = SpscRingBuffer::<u32, 3>::new(); // should fail
    // }
}
