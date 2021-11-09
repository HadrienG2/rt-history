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
    written: Atomic<usize>,

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
        assert!(input.len() <= data_len);

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
        if old_writing_idx <= new_writing_idx {
            for (src, dst) in input
                .iter()
                .zip(&self.0.data[old_writing_idx..new_writing_idx])
            {
                // TODO: Check codegen. If it's bad, try volatile memcpy as an
                //       alternative, but put this behind a feature flag because
                //       it assumes backend store/load races are not UB.
                dst.store(*src, Ordering::Relaxed);
            }
        } else {
            let first_half = &self.0.data[old_writing_idx..];
            let second_half = &self.0.data[..new_writing_idx];
            for (src, dst) in input.iter().zip(first_half.iter().chain(second_half)) {
                // TODO: See above
                dst.store(*src, Ordering::Relaxed);
            }
        }

        // Notify the consumer that new data has been published, make sure that
        // this write is ordered after previous data writes
        if cfg!(debug_assertions) {
            assert_eq!(self.0.written.load(Ordering::Relaxed), old_writing);
        }
        self.0.written.store(new_writing, Ordering::Release);
    }
}

/// Consumer interface to the history log
pub struct Output<T: Copy + Sync>(Arc<SharedState<T>>);
//
impl<T: Copy + Sync> Output<T> {
    /// Check the current timestamp
    ///
    /// The wrapping_sub of two readings of clock() tells you how many new
    /// entries have been written into the history between these readings.
    ///
    pub fn clock(&self) -> usize {
        atomic::fence(Ordering::Acquire);
        self.0.written.load(Ordering::Acquire)
    }

    /// Read the last N entries, tell if a buffer overrun occured
    ///
    /// In the event of a negative result, output data will be corrupt in an
    /// unspecified way. This means that the inner circular buffer is too small
    /// and its size should be increased.
    ///
    pub fn read_and_check_overrun(&self, output: &mut [T]) -> bool {
        // Check that the request makes sense
        let data_len = self.0.data_len();
        assert!(output.len() <= data_len);

        // Check the timestamp of the last published data point, and make sure
        // this read is ordered before subsequent data reads
        let last_written = self.0.written.load(Ordering::Acquire);

        // Deduce the timestamp of the first data point that we're interested in
        let first_written = last_written.wrapping_sub(output.len());

        // Wrap old and new timestamps into circular buffer range
        let last_written_idx = last_written % data_len;
        let first_written_idx = first_written % data_len;

        // Perform the data reads
        if first_written_idx <= last_written_idx {
            for (src, dst) in self.0.data[first_written_idx..last_written_idx]
                .iter()
                .zip(output.iter_mut())
            {
                // TODO: See above
                *dst = src.load(Ordering::Relaxed);
            }
        } else {
            let first_half = &self.0.data[first_written_idx..];
            let second_half = &self.0.data[..last_written_idx];
            for (src, dst) in first_half.iter().chain(second_half).zip(output.iter_mut()) {
                // TODO: See above
                *dst = src.load(Ordering::Relaxed);
            }
        }

        // Make sure the producer did not concurrently overwrite our data.
        // This overrun check must be ordered after the previous data reads.
        atomic::fence(Ordering::Acquire);
        let last_writing = self.0.writing.load(Ordering::Relaxed);
        if first_written <= last_written {
            last_writing >= first_written && last_writing < last_written
        } else {
            last_writing >= first_written || last_writing < last_written
        }
    }
}

/// A realtime-safe (bounded wait-free) history log
pub struct RTHistory<T: Copy + Sync>(Arc<SharedState<T>>);
//
impl<T: Copy + Default + Sync> RTHistory<T> {
    /// Build a history log that can hold a certain number of entries
    ///
    /// To avoid data corruption (buffer overruns), the log must be larger than
    /// the size of the largest read to be performed by the consumer. If you are
    /// not tight on RAM a safe starting point is to make it three times larger.
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
            written: Atomic::<usize>::new(0),
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
    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}
