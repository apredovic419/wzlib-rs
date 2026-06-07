use std::hint::black_box;
use std::io::Cursor;
use std::sync::Arc;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use wzlib_rs::wz::image_writer::write_image;
use wzlib_rs::{
    parse_wz_image, parse_wz_image_lazy, CanvasData, WzBinaryReader, WzBinaryWriter, WzHeader,
    WzPngFormat, WzProperty,
};

/// Serialize a synthetic IMG: `num_canvas` Canvas frames, each carrying a
/// `blob_size`-byte compressed-pixel payload plus a few typical child props.
fn build_image_bytes(num_canvas: usize, blob_size: usize) -> Vec<u8> {
    let blob = vec![0x5Au8; blob_size];
    let mut props = Vec::with_capacity(num_canvas);
    for i in 0..num_canvas {
        let canvas = WzProperty::Canvas {
            width: 64,
            height: 64,
            format: WzPngFormat::Bgra8888,
            scale: 0,
            properties: vec![
                ("origin".to_string(), WzProperty::Vector { x: 0, y: 0 }),
                ("delay".to_string(), WzProperty::Int(100)),
                ("z".to_string(), WzProperty::Int(0)),
            ],
            png_data: CanvasData::Loaded(blob.clone()),
        };
        props.push((i.to_string(), canvas));
    }
    let mut writer = WzBinaryWriter::new(Cursor::new(Vec::new()), [0; 4], WzHeader::dummy(0));
    write_image(&mut writer, &props).unwrap();
    writer.into_inner().into_inner()
}

fn bench_parse(c: &mut Criterion) {
    for &(n, sz) in &[(50usize, 4096usize), (200, 8192)] {
        let data = build_image_bytes(n, sz);
        let src: Arc<[u8]> = Arc::from(data.as_slice());
        let mut group = c.benchmark_group(format!("parse_image/{n}x{}KB", sz / 1024));
        group.throughput(Throughput::Bytes(data.len() as u64));

        group.bench_function(BenchmarkId::new("eager", "copy"), |b| {
            b.iter(|| {
                let mut reader = WzBinaryReader::new(
                    Cursor::new(data.as_slice()),
                    [0; 4],
                    WzHeader::dummy(data.len() as u64),
                    0,
                );
                black_box(parse_wz_image(&mut reader).unwrap());
            })
        });
        group.bench_function(BenchmarkId::new("lazy", "ref"), |b| {
            b.iter(|| {
                let mut reader = WzBinaryReader::new(
                    Cursor::new(data.as_slice()),
                    [0; 4],
                    WzHeader::dummy(data.len() as u64),
                    0,
                );
                black_box(parse_wz_image_lazy(&mut reader, src.clone()).unwrap());
            })
        });
        group.finish();
    }
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
