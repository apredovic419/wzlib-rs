use std::hint::black_box;
use std::io::Cursor;

use criterion::{criterion_group, criterion_main, Criterion};

use wzlib_rs::{WzBinaryReader, WzHeader, WzKey};

const GMS_IV: [u8; 4] = [0x4D, 0x23, 0xC7, 0x2B];

/// Encode an ASCII string the way `read_wz_ascii_string` expects: negative
/// indicator (or `i8::MIN` + i32 length for len ≥ 128), then `byte ^ mask ^ key`.
fn encode_ascii(bytes: &[u8]) -> Vec<u8> {
    let len = bytes.len();
    let mut key = WzKey::new(GMS_IV);
    key.ensure_size(len);
    let mut out = Vec::new();
    if len >= 128 {
        out.push(i8::MIN as u8);
        out.extend_from_slice(&(len as i32).to_le_bytes());
    } else {
        out.push((-(len as i32) as i8) as u8);
    }
    let mut mask: u8 = 0xAA;
    for (i, &b) in bytes.iter().enumerate() {
        out.push(b ^ mask ^ key.get(i));
        mask = mask.wrapping_add(1);
    }
    out
}

fn bench_string(c: &mut Criterion) {
    let mut group = c.benchmark_group("string_decrypt");
    for &len in &[8usize, 32, 120, 4096] {
        let encoded = encode_ascii(&vec![b'a'; len]);
        group.bench_function(format!("ascii_{len}"), |b| {
            b.iter(|| {
                let mut reader = WzBinaryReader::new(
                    Cursor::new(encoded.as_slice()),
                    GMS_IV,
                    WzHeader::dummy(encoded.len() as u64),
                    0,
                );
                black_box(reader.read_wz_string().unwrap());
            })
        });
    }
    group.finish();
}

criterion_group!(benches, bench_string);
criterion_main!(benches);
