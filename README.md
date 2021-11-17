# rt-history: An RT-safe history log with error checking

[![On crates.io](https://img.shields.io/crates/v/rt-history.svg)](https://crates.io/crates/rt-history)
[![On docs.rs](https://docs.rs/rt-history/badge.svg)](https://docs.rs/rt-history/)
[![Continuous Integration](https://github.com/HadrienG2/rt-history/workflows/Continuous%20Integration/badge.svg)](https://github.com/HadrienG2/rt-history/actions?query=workflow%3A%22Continuous+Integration%22)
![Requires rustc 1.36+](https://img.shields.io/badge/rustc-1.56+-red.svg)

This is a bounded wait-free thread synchronization primitive which allows
you to record the time evolution of some data on one thread and be able to
query the last N data points on other threads.

By definition of wait-free synchronization, the producer and consumer
threads cannot wait for each other, so in bad conditions (too small a buffer
size, too low a thread's priority, too loaded a system...), two race
conditions can occur:

- The producer can be too fast with respect to consumers, and overwrite
  historical data which a consumer was still in the process of reading.
  This buffer overrun scenario is reported, along with the degree of overrun
  that occurred, which can be used to guide system parameter adjustments so
  that the error stops occurring.
- The producer can be too slow with respect to consumers, and fail to write
  a sufficient amount of new data inbetween two consumer readouts. The
  precise definition of this buffer underrun error is workload-dependent, so
  we provide the right tool to detect it in the form of a latest data point
  timestamp, but we do not handle it ourselves.

```rust
let (mut input, output) = RTHistory::<u8>::new(8).split();

let in_buf = [1, 2, 3];
input.write(&in_buf[..]);

let mut out_buf = [0; 3];
assert_eq!(output.read(&mut out_buf[..]), Ok(3));
assert_eq!(in_buf, out_buf);
```