use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

use wzlib_rs::{compress_png_data, decode_pixels, decompress_png_data, WzPngFormat};

fn bench_decode(c: &mut Criterion) {
    let (w, h) = (256u32, 256u32);

    // Pseudo-pixel data (content is irrelevant to conversion cost).
    let raw_8888: Vec<u8> = (0..(w * h * 4)).map(|i| (i % 251) as u8).collect();
    let raw_4444: Vec<u8> = (0..(w * h * 2)).map(|i| (i % 251) as u8).collect();
    let compressed_8888 = compress_png_data(&raw_8888).unwrap();

    let mut group = c.benchmark_group("canvas_decode/256x256");

    group.throughput(Throughput::Bytes(raw_8888.len() as u64));
    group.bench_function("zlib_decompress", |b| {
        b.iter(|| black_box(decompress_png_data(black_box(&compressed_8888), None).unwrap()))
    });
    group.bench_function("convert_bgra8888", |b| {
        b.iter(|| {
            black_box(decode_pixels(black_box(&raw_8888), w, h, WzPngFormat::Bgra8888).unwrap())
        })
    });
    group.bench_function("decompress+convert_bgra8888", |b| {
        b.iter(|| {
            let raw = decompress_png_data(black_box(&compressed_8888), None).unwrap();
            black_box(decode_pixels(&raw, w, h, WzPngFormat::Bgra8888).unwrap())
        })
    });

    group.throughput(Throughput::Bytes(raw_4444.len() as u64));
    group.bench_function("convert_bgra4444", |b| {
        b.iter(|| {
            black_box(decode_pixels(black_box(&raw_4444), w, h, WzPngFormat::Bgra4444).unwrap())
        })
    });

    group.finish();
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
