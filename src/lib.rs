//! TODO: Add toplevel docs

use atomic::{self, Atomic, Ordering};
use std::sync::Arc;

/// Data that is shared between the producer and the consumer
struct SharedState<T: Copy + Sync> {
    /// Circular buffer
    data: Box<[Atomic<T>]>,

    /// data.len() will always be a power of 2, with this exponent, and the
    /// compiler can use this knowledge to compute modulos much faster
    data_len_pow2: u32,

    /// Timestamp of the last entry that has been published
    readable: Atomic<usize>,

    /// Timestamp of the last entry that has been or is being overwritten
    writing: Atomic<usize>,
}
//
impl<T: Copy + Sync> SharedState<T> {
    /// Length of the inner circular buffer
    #[inline(always)]
    fn data_len(&self) -> usize {
        let data_len = 1usize.pow(self.data_len_pow2);
        debug_assert_eq!(self.data.len(), data_len);
        data_len
    }
}

/// Producer interface to the history log
pub struct Input<T: Copy + Sync>(Arc<SharedState<T>>);
//
impl<T: Copy + Sync> Input<T> {
    /// Add some entries to the history log
    pub fn write(&mut self, input: &[T]) {
        // Check that the request makes sense
        let data_len = self.0.data_len();
        assert!(
            input.len() <= data_len,
            "History is shorted than provided input"
        );

        // Notify the consumer that we are writing new data
        let old_writing = self.0.writing.load(Ordering::Relaxed);
        let new_writing = old_writing.wrapping_add(input.len());
        self.0.writing.store(new_writing, Ordering::Relaxed);

        // Make sure that this notification is not reordered after data writes
        atomic::fence(Ordering::Release);

        // Wrap old and new writing timestamps into circular buffer range
        let old_writing_idx = old_writing % data_len;
        let new_writing_idx = new_writing % data_len;

        // Perform the data writes
        let first_half = &self.0.data[old_writing_idx..];
        let second_half = &self.0.data[..new_writing_idx];
        for (src, dst) in input.iter().zip(first_half.iter().chain(second_half)) {
            // TODO: Check codegen. If it's bad, try volatile memcpy as an
            //       alternative, but put this behind a feature flag because
            //       it assumes backend store/load races are not UB.
            dst.store(*src, Ordering::Relaxed);
        }

        // Notify the consumer that new data has been published, make sure that
        // this write is ordered after previous data writes
        if cfg!(debug_assertions) {
            assert_eq!(self.0.readable.load(Ordering::Relaxed), old_writing);
        }
        self.0.readable.store(new_writing, Ordering::Release);
    }
}

/// Number of entries that the producer wrote so far
///
/// Differences between two consecutive readouts of this counter can be used to
/// tell how many new entries arrived between two readouts, and detect buffer
/// underruns where the producer wrote too few data. How few is too few is
/// workload-dependent, so we do not implement this logic ourselves.
///
/// This counter may wrap around from time to time, every couple of hours for a
/// mono audio stream on a 32-bit CPU, so use wrapping_sub() for deltas.
///
pub type Clock = usize;

/// Indication that a buffer overrun occured
///
/// This means that the producer wrote over the data that the consumer was in
/// the process of reading. Possible fixes include making the buffer larger,
/// speeding up the consumer, and slowing down the producer, in order of
/// decreasing preference.
///
// TODO: Use thiserror to make this a real error type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overrun {
    /// Number of entries that were fully written by the producer at the time
    /// where the consumer started reading out data
    pub clock: Clock,

    /// Number of early history entries that were overwritten by the producer
    /// while the consumer was in the process of reading out data.
    ///
    /// This can be higher than the number of entries which the consumer
    /// requested. The intent is to provide a quantitative indication of how
    /// late the consumer is with respect to the producer, so that the buffer
    /// size can be increased by a matching amount if need be.
    ///
    pub excess_entries: Clock,
}

/// Consumer interface to the history log
#[derive(Clone)]
pub struct Output<T: Copy + Sync>(Arc<SharedState<T>>);
//
impl<T: Copy + Sync> Output<T> {
    /// Read the last N entries, providing the associated timestamp and checking
    /// for overruns
    pub fn read(&self, output: &mut [T]) -> Result<Clock, Overrun> {
        // Check that the request makes sense
        let data_len = self.0.data_len();
        assert!(
            output.len() <= data_len,
            "History is shorter than requested output"
        );

        // Check the timestamp of the last published data point, and make sure
        // this read is ordered before subsequent data reads
        let last_readable = self.0.readable.load(Ordering::Acquire);

        // Deduce the timestamp of the first data point that we're interested in
        let first_readable = last_readable.wrapping_sub(output.len());

        // Wrap old and new timestamps into circular buffer range
        let last_readable_idx = last_readable % data_len;
        let first_readable_idx = first_readable % data_len;

        // Perform the data reads
        let first_half = &self.0.data[first_readable_idx..];
        let second_half = &self.0.data[..last_readable_idx];
        for (src, dst) in first_half.iter().chain(second_half).zip(output.iter_mut()) {
            // TODO: See above
            *dst = src.load(Ordering::Relaxed);
        }

        // Make sure the producer did not concurrently overwrite our data.
        // This overrun check must be ordered after the previous data reads.
        atomic::fence(Ordering::Acquire);
        let last_writing = self.0.writing.load(Ordering::Relaxed);
        let excess_entries = last_writing
            .wrapping_sub(first_readable)
            .saturating_sub(data_len);

        // Produce final result
        if excess_entries > 0 {
            Err(Overrun {
                clock: last_readable,
                excess_entries,
            })
        } else {
            Ok(last_readable)
        }
    }
}

/// A realtime-safe (bounded wait-free) history log
pub struct RTHistory<T: Copy + Sync>(Arc<SharedState<T>>);
//
impl<T: Copy + Default + Sync> RTHistory<T> {
    /// Build a history log that can hold a certain number of entries
    ///
    /// To avoid data corruption (buffer overruns), the log must be
    /// significantly larger than the size of the largest read to be performed
    /// by the consumer. If you are not tight on RAM, a safe starting point is
    /// to make it three times as large.
    ///
    /// For efficiency reason, the actual history buffer size will be rounded to
    /// the next power of two.
    ///
    pub fn new(min_entries: usize) -> Self {
        assert!(
            Atomic::<T>::is_lock_free(),
            "Cannot build a lock-free history log if type T does not have lock-free atomics"
        );
        let data_len = min_entries.next_power_of_two();
        let data_len_pow2 = data_len.trailing_zeros();
        Self(Arc::new(SharedState {
            data: std::iter::repeat_with(Atomic::<T>::default)
                .take(data_len)
                .collect(),
            data_len_pow2,
            readable: Atomic::<usize>::new(0),
            writing: Atomic::<usize>::new(0),
        }))
    }
}
//
impl<T: Copy + Sync> RTHistory<T> {
    /// Split the history log into a producer and consumer interface
    pub fn split(self) -> (Input<T>, Output<T>) {
        (Input(self.0.clone()), Output(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quickcheck::TestResult;
    use quickcheck_macros::quickcheck;
    use std::panic::{self, AssertUnwindSafe};

    macro_rules! validate_min_entries {
        ($min_entries:expr) => {
            if $min_entries > 1024 * 1024 * 1024 {
                return TestResult::discard();
            }
        };
    }

    #[quickcheck]
    fn new_split_clock(min_entries: usize) -> TestResult {
        // Reject impossibly large buffer configuraitons
        validate_min_entries!(min_entries);

        // Check initial state
        let history = RTHistory::<f32>::new(min_entries);
        assert!(history
            .0
            .data
            .iter()
            .map(|x| x.load(Ordering::Relaxed))
            .all(|f| f == 0.0));
        assert_eq!(history.0.data_len(), min_entries.next_power_of_two());
        assert_eq!(history.0.data_len(), history.0.data.len());
        assert_eq!(history.0.readable.load(Ordering::Relaxed), 0);
        assert_eq!(history.0.writing.load(Ordering::Relaxed), 0);

        // Split producer and consumer interface
        let (input, output) = history.split();

        // Check that they point to the same thing
        assert_eq!(&*input.0 as *const _, &*output.0 as *const _);

        TestResult::passed()
    }

    #[quickcheck]
    fn writes_and_read(
        min_entries: usize,
        write1: Vec<u32>,
        write2: Vec<u32>,
        read_size: usize,
    ) -> TestResult {
        // Reject impossibly large buffer configurations
        validate_min_entries!(min_entries);
        validate_min_entries!(read_size);

        // Set up a history log
        let (mut input, output) = RTHistory::<u32>::new(min_entries).split();

        // Attempt a write, which will fail if and only if the input data is
        // larger than the history log's inner buffer.
        macro_rules! checked_write {
            ($write:expr, $input:expr) => {
                if let Err(_) = panic::catch_unwind(AssertUnwindSafe(|| $input.write(&$write[..])))
                {
                    assert!($write.len() > $input.0.data_len());
                    return TestResult::passed();
                }
                assert!($write.len() <= $input.0.data_len());
            };
        }
        checked_write!(write1, input);

        // Check the final buffer state
        assert!(input
            .0
            .data
            .iter()
            .take(write1.len())
            .zip(&write1)
            .all(|(a, &x)| a.load(Ordering::Relaxed) == x));
        assert!(input
            .0
            .data
            .iter()
            .skip(write1.len())
            .all(|a| a.load(Ordering::Relaxed) == 0));
        assert_eq!(input.0.readable.load(Ordering::Relaxed), write1.len());
        assert_eq!(input.0.writing.load(Ordering::Relaxed), write1.len());

        // Perform another write, check state again
        checked_write!(write2, input);
        let new_readable = write1.len() + write2.len();
        let data_len = input.0.data_len();
        let overwritten = new_readable.saturating_sub(data_len);
        assert!(input
            .0
            .data
            .iter()
            .take(overwritten)
            .zip(write2[write2.len() - overwritten..].iter())
            .all(|(a, &x2)| a.load(Ordering::Relaxed) == x2));
        assert!(input
            .0
            .data
            .iter()
            .skip(overwritten)
            .take(write1.len().saturating_sub(overwritten))
            .zip(write1.iter().skip(overwritten))
            .all(|(a, &x1)| a.load(Ordering::Relaxed) == x1));
        assert!(input
            .0
            .data
            .iter()
            .skip(write1.len())
            .take(write2.len())
            .zip(&write2)
            .all(|(a, &x2)| a.load(Ordering::Relaxed) == x2));
        assert!(input
            .0
            .data
            .iter()
            .skip(new_readable)
            .all(|a| a.load(Ordering::Relaxed) == 0));
        assert_eq!(input.0.readable.load(Ordering::Relaxed), new_readable);
        assert_eq!(input.0.writing.load(Ordering::Relaxed), new_readable);

        // Back up current ring buffer state
        let data_backup = input
            .0
            .data
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect::<Box<[_]>>();

        // Read some data back and make sure no overrun happened (it is
        // impossible in single-threaded code)
        let mut read = vec![0; read_size];
        match panic::catch_unwind(AssertUnwindSafe(|| output.read(&mut read[..]))) {
            Err(_) => {
                assert!(read.len() > output.0.data_len());
                return TestResult::passed();
            }
            Ok(result) => {
                assert!(read.len() <= output.0.data_len());
                assert_eq!(result, Ok(new_readable));
            }
        }

        // Make sure that the "read" did not alter the history data
        assert!(input
            .0
            .data
            .iter()
            .zip(data_backup.iter().copied())
            .all(|(a, x)| a.load(Ordering::Relaxed) == x));
        assert_eq!(input.0.readable.load(Ordering::Relaxed), new_readable);
        assert_eq!(input.0.writing.load(Ordering::Relaxed), new_readable);

        // Check that the collected output is correct
        assert!(read
            .iter()
            .rev()
            .zip(
                write2
                    .iter()
                    .rev()
                    .chain(write1.iter().rev())
                    .chain(std::iter::repeat(&0))
            )
            .all(|(&x, &y)| x == y));

        TestResult::passed()
    }

    // TODO: Add ignored test of behavior in a multi-threaded scenario
    //       Pick a scenario where overrun is very likely (write size = buffer
    //       size) and check that overrun is properly detected, in the sense
    //       that we do see some overrun, and when we don't see it the data is
    //       correct. Use a regular pattern in the data (e.g. sequence of u64s)
    //       to make the data check easier. Check with multiple consumers.
}
