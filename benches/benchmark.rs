use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rt_history::RTHistory;

fn from_elem(c: &mut Criterion) {
    type T = f32;
    const HISTORY_SIZE: usize = 2048;
    let (mut input, output) = RTHistory::<T>::new(HISTORY_SIZE).split();

    let [mut buf, mut buf2] = [[T::default(); HISTORY_SIZE]; 2];
    const BUF_SIZES: [usize; 12] = [1, 2, 4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048];

    {
        let mut group = c.benchmark_group("write");
        for size in BUF_SIZES {
            group.throughput(Throughput::Bytes(
                (2 * std::mem::size_of::<T>() * size) as u64,
            ));
            group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
                b.iter(|| input.write(&buf[..size]));
            });
        }
        group.finish();
    }

    {
        let mut group = c.benchmark_group("read");
        for size in BUF_SIZES {
            group.throughput(Throughput::Bytes(
                (2 * std::mem::size_of::<T>() * size) as u64,
            ));
            group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
                b.iter(|| output.read(&mut buf[..size]));
            });
        }
        group.finish();
    }

    {
        let mut group = c.benchmark_group("contended_write");
        for size in BUF_SIZES {
            group.throughput(Throughput::Bytes(
                (2 * std::mem::size_of::<T>() * size) as u64,
            ));
            testbench::run_under_contention(
                || output.read(&mut buf2[..size]),
                || {
                    group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
                        b.iter(|| input.write(&buf[..size]));
                    });
                },
            );
        }
        group.finish();
    }

    {
        let mut group = c.benchmark_group("contended_read");
        for size in BUF_SIZES {
            group.throughput(Throughput::Bytes(
                (2 * std::mem::size_of::<T>() * size) as u64,
            ));
            testbench::run_under_contention(
                || input.write(&buf2[..size]),
                || {
                    group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &size| {
                        b.iter(|| output.read(&mut buf[..size]));
                    });
                },
            );
        }
        group.finish();
    }
}

criterion_group!(benches, from_elem);
criterion_main!(benches);
