//! Image decoding and encoding — pixel format conversion, DXT decompression, zlib compression.
//!
//! Ported from MapleLib's `PngUtility.cs` and `WzPngProperty.cs`.
//! Decoders output RGBA8888; encoders accept RGBA8888 and produce WZ-native formats.

pub mod dxt;
pub mod encode;
pub mod pixel;

use crate::wz::error::{WzError, WzResult};
use crate::wz::types::WzPngFormat;

// Tolerates short data by zero-padding (matches C# pre-allocated buffer behavior).
pub fn decode_pixels(
    raw: &[u8],
    width: u32,
    height: u32,
    format: WzPngFormat,
) -> WzResult<Vec<u8>> {
    use std::borrow::Cow;

    let expected = format.raw_data_size(width, height);

    let data: Cow<[u8]> = if expected > 0 && raw.len() < expected {
        let mut padded = raw.to_vec();
        padded.resize(expected, 0);
        Cow::Owned(padded)
    } else {
        Cow::Borrowed(raw)
    };

    let pixel_count = (width * height) as usize;
    match format {
        //   1 — BGRA4444
        WzPngFormat::Bgra4444 => pixel::bgra4444_to_rgba(&data, pixel_count),
        //   2 — BGRA8888
        WzPngFormat::Bgra8888 => pixel::bgra8888_to_rgba(&data, pixel_count),
        //   3 — DXT3 grayscale  /  1026 — DXT3 colored
        WzPngFormat::Dxt3Grayscale | WzPngFormat::Dxt3 => {
            dxt::decompress_dxt3(&data, width, height)
        }
        // 257 — ARGB1555
        WzPngFormat::Argb1555 => pixel::argb1555_to_rgba(&data, pixel_count),
        // 513 — RGB565
        WzPngFormat::Rgb565 => pixel::rgb565_to_rgba(&data, pixel_count),
        // 517 — RGB565 block
        WzPngFormat::Rgb565Block => pixel::rgb565_block_to_rgba(&data, width, height),
        // 769 — R16
        WzPngFormat::R16 => pixel::r16_to_rgba(&data, pixel_count),
        // 2050 — DXT5
        WzPngFormat::Dxt5 => dxt::decompress_dxt5(&data, width, height),
        // 2304 — A8
        WzPngFormat::A8 => pixel::a8_to_rgba(&data, pixel_count),
        // 2562 — RGBA1010102
        WzPngFormat::Rgba1010102 => pixel::rgba1010102_to_rgba(&data, pixel_count),
        // 4097 — DXT1/BC1
        WzPngFormat::Dxt1 => dxt::decompress_dxt1(&data, width, height),
        // 4098 — BC7
        WzPngFormat::Bc7 => dxt::decompress_bc7(&data, width, height),
        // 4100 — RGBA32Float
        WzPngFormat::Rgba32Float => pixel::rgba32float_to_rgba(&data, pixel_count),

        WzPngFormat::Unknown(id) => Err(WzError::UnsupportedPngFormat(id)),
    }
}

/// Decode a canvas's decompressed bytes to RGBA8888, honoring the WZ scale
/// exponent (`format2`).
///
/// When `scale == 0` this is exactly [`decode_pixels`]. When `scale > 0` the
/// pixel data is stored at `(width >> scale) × (height >> scale)` (the reduced
/// resolution the compressed `raw` actually contains) and is then
/// nearest-neighbor upscaled to the full `(width, height)`. This matches the
/// official v83 client, where solid-color / gradient fill layers (sky, water,
/// flat backgrounds) are stored downscaled to save space.
pub fn decode_canvas_pixels(
    raw: &[u8],
    width: u32,
    height: u32,
    scale: u8,
    format: WzPngFormat,
) -> WzResult<Vec<u8>> {
    if scale == 0 {
        return decode_pixels(raw, width, height, format);
    }
    let sw = (width >> scale).max(1);
    let sh = (height >> scale).max(1);
    let small = decode_pixels(raw, sw, sh, format)?;
    Ok(nearest_upscale(&small, sw, sh, width, height))
}

/// Nearest-neighbor upscale of an RGBA8888 buffer from `(sw, sh)` to `(dw, dh)`.
fn nearest_upscale(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dw as usize) * (dh as usize) * 4];
    if sw == 0 || sh == 0 {
        return out;
    }
    for y in 0..dh {
        let sy = ((y * sh) / dh).min(sh - 1);
        for x in 0..dw {
            let sx = ((x * sw) / dw).min(sw - 1);
            let si = ((sy * sw + sx) as usize) * 4;
            let di = ((y * dw + x) as usize) * 4;
            if si + 4 <= src.len() {
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    out
}

/// Inflate `input`, tolerating a stream that never reaches its terminator.
///
/// WZ canvas blobs are stored as a raw inflate window whose last deflate block
/// is not marked BFINAL and which carries no adler32 trailer — the archive
/// records the byte length instead, so the writer never had to close the
/// stream. Reference C zlib therefore reports `Z_BUF_ERROR` / "incomplete or
/// truncated stream" on them even though every pixel byte is present
/// (a 26×16 BGRA4444 face inflates to exactly 832 = w·h·2 bytes and then stops).
///
/// `miniz_oxide` 0.8.x silently accepted that and returned the bytes; 0.9
/// aligned with C zlib and surfaces the error, so the previous
/// `Read::read_to_end` here threw away fully-decoded pixel data for *every*
/// canvas. Driving the inflater directly lets us tell "ran out of input after
/// producing N bytes" (fine — return them) apart from a real data error or a
/// stream that produced nothing (still an error).
fn inflate_lenient(input: &[u8], zlib_header: bool) -> WzResult<Vec<u8>> {
    use flate2::{Decompress, FlushDecompress, Status};

    const MIN_CHUNK: usize = 4 * 1024;
    // Guess ~4× the compressed size, then grow geometrically. The cap keeps a
    // pathologically large blob from reserving hundreds of MB up front.
    const MAX_PREALLOC: usize = 16 * 1024 * 1024;

    let mut inflater = Decompress::new(zlib_header);
    let mut output: Vec<u8> =
        Vec::with_capacity(input.len().saturating_mul(4).clamp(MIN_CHUNK, MAX_PREALLOC));
    let mut stream_end = false;

    loop {
        if output.len() == output.capacity() {
            output.reserve(output.capacity().max(MIN_CHUNK));
        }

        let consumed = (inflater.total_in() as usize).min(input.len());
        let before_in = inflater.total_in();
        let before_out = inflater.total_out();

        match inflater.decompress_vec(&input[consumed..], &mut output, FlushDecompress::None) {
            Ok(Status::StreamEnd) => {
                stream_end = true;
                break;
            }
            // No progress possible: the inflater wants more input that does not
            // exist (the unterminated-stream case) — stop and keep what we got.
            Ok(Status::BufError) => break,
            Ok(Status::Ok) => {
                if inflater.total_in() == before_in && inflater.total_out() == before_out {
                    break; // defensive: never spin on a no-op round
                }
            }
            Err(e) => return Err(WzError::DecompressionFailed(e.to_string())),
        }
    }

    if !stream_end && output.is_empty() {
        return Err(WzError::DecompressionFailed(
            "incomplete deflate stream (no data decoded)".into(),
        ));
    }

    Ok(output)
}

pub fn decompress_png_data(compressed: &[u8], wz_key: Option<&[u8]>) -> WzResult<Vec<u8>> {
    use std::io::Cursor;

    if compressed.len() < 2 {
        return Err(WzError::DecompressionFailed("Data too short".into()));
    }

    let is_zlib = matches!(
        (compressed[0], compressed[1]),
        (0x78, 0x9C) | (0x78, 0xDA) | (0x78, 0x01) | (0x78, 0x5E)
    );

    if is_zlib {
        inflate_lenient(compressed, true)
    } else if let Some(key) = wz_key {
        // list.wz encrypted block format: [blocksize:i32][XOR'd bytes]... → zlib after decrypt
        let mut cursor = Cursor::new(compressed);
        let mut decrypted = Vec::new();
        let end = compressed.len() as u64;

        while cursor.position() < end {
            let mut size_buf = [0u8; 4];
            std::io::Read::read_exact(&mut cursor, &mut size_buf)
                .map_err(|e| WzError::DecompressionFailed(e.to_string()))?;
            let block_size = i32::from_le_bytes(size_buf) as usize;

            let mut block = vec![0u8; block_size];
            std::io::Read::read_exact(&mut cursor, &mut block)
                .map_err(|e| WzError::DecompressionFailed(e.to_string()))?;

            for i in 0..block_size {
                if i < key.len() {
                    block[i] ^= key[i];
                }
            }
            decrypted.extend_from_slice(&block);
        }

        if decrypted.len() < 2 {
            return Err(WzError::DecompressionFailed(
                "Decrypted list.wz data too short".into(),
            ));
        }
        inflate_lenient(&decrypted[2..], false) // skip 2-byte zlib header
    } else {
        inflate_lenient(compressed, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    // ── decode_pixels ──────────────────────────────────────────────

    #[test]
    fn test_decode_pixels_bgra8888_white() {
        // 2×2 BGRA8888 all-white: B=FF, G=FF, R=FF, A=FF per pixel
        let raw = vec![0xFF; 2 * 2 * 4]; // 16 bytes
        let result = decode_pixels(&raw, 2, 2, WzPngFormat::Bgra8888).unwrap();
        assert_eq!(result.len(), 16); // 2*2*4
                                      // After BGRA→RGBA swap, still all 0xFF
        assert!(result.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn test_decode_pixels_a8() {
        // 2×2 A8: alpha values [0, 128, 255, 64]
        let raw = vec![0, 128, 255, 64];
        let result = decode_pixels(&raw, 2, 2, WzPngFormat::A8).unwrap();
        assert_eq!(result.len(), 16);
        // Pixel 0: R=255, G=255, B=255, A=0
        assert_eq!(&result[0..4], &[255, 255, 255, 0]);
        // Pixel 2: R=255, G=255, B=255, A=255
        assert_eq!(&result[8..12], &[255, 255, 255, 255]);
    }

    // ── decode_canvas_pixels (scale exponent) ─────────────────────────

    #[test]
    fn test_decode_canvas_scale0_equals_decode_pixels() {
        let raw = vec![0xFF; 2 * 2 * 4];
        let a = decode_canvas_pixels(&raw, 2, 2, 0, WzPngFormat::Bgra8888).unwrap();
        let b = decode_pixels(&raw, 2, 2, WzPngFormat::Bgra8888).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_decode_canvas_scale_upscales_nearest() {
        // 2×2 source stored, scale=1 → upscale to 4×4. BGRA8888 distinct pixels:
        // (0,0)=red (1,0)=green (0,1)=blue (1,1)=white. Bytes are B,G,R,A.
        let raw = vec![
            0x00, 0x00, 0xFF, 0xFF, // px(0,0) red
            0x00, 0xFF, 0x00, 0xFF, // px(1,0) green
            0xFF, 0x00, 0x00, 0xFF, // px(0,1) blue
            0xFF, 0xFF, 0xFF, 0xFF, // px(1,1) white
        ];
        let out = decode_canvas_pixels(&raw, 4, 4, 1, WzPngFormat::Bgra8888).unwrap();
        assert_eq!(out.len(), 4 * 4 * 4);
        let px = |x: usize, y: usize| {
            let i = (y * 4 + x) * 4;
            &out[i..i + 4]
        };
        // top-left 2×2 block = red (RGBA after BGRA swap)
        assert_eq!(px(0, 0), &[0xFF, 0x00, 0x00, 0xFF]);
        assert_eq!(px(1, 1), &[0xFF, 0x00, 0x00, 0xFF]);
        // top-right 2×2 block = green
        assert_eq!(px(2, 0), &[0x00, 0xFF, 0x00, 0xFF]);
        assert_eq!(px(3, 1), &[0x00, 0xFF, 0x00, 0xFF]);
        // bottom-left = blue, bottom-right = white
        assert_eq!(px(0, 2), &[0x00, 0x00, 0xFF, 0xFF]);
        assert_eq!(px(3, 3), &[0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn test_decode_canvas_midforest_shape() {
        // Real midForest case: RGB565 (513) stored at 8×8, scale=4 → 128×128.
        // 8×8 RGB565 = 128 bytes; upscaled RGBA = 128*128*4.
        let raw = vec![0u8; 8 * 8 * 2];
        let out = decode_canvas_pixels(&raw, 128, 128, 4, WzPngFormat::Rgb565).unwrap();
        assert_eq!(out.len(), 128 * 128 * 4);
    }

    #[test]
    fn test_decode_pixels_unknown_format_error() {
        let raw = vec![0; 16];
        let err = decode_pixels(&raw, 2, 2, WzPngFormat::Unknown(999)).unwrap_err();
        matches!(err, WzError::UnsupportedPngFormat(999));
    }

    #[test]
    fn test_decode_pixels_short_data_zero_padded() {
        // BGRA8888 2×2 expects 16 bytes, provide only 8 → zero-padded
        let raw = vec![0xFF; 8]; // only half the data
        let result = decode_pixels(&raw, 2, 2, WzPngFormat::Bgra8888).unwrap();
        assert_eq!(result.len(), 16);
        // First 2 pixels have data, last 2 are zero-padded
        // Pixel 0: B=FF,G=FF,R=FF,A=FF → R=FF,G=FF,B=FF,A=FF
        assert_eq!(&result[0..4], &[0xFF, 0xFF, 0xFF, 0xFF]);
        // Pixel 2 (padded zeros): B=0,G=0,R=0,A=0 → R=0,G=0,B=0,A=0
        assert_eq!(&result[8..12], &[0, 0, 0, 0]);
    }

    // ── decompress_png_data ────────────────────────────────────────

    #[test]
    fn test_decompress_png_data_zlib() {
        // Compress known data with zlib, then decompress
        let original = b"Hello, WZ world!";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_png_data(&compressed, None).unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_png_data_too_short() {
        for input in [&[][..], &[0x78][..]] {
            let err = decompress_png_data(input, None).unwrap_err();
            assert!(
                matches!(err, WzError::DecompressionFailed(_)),
                "expected DecompressionFailed for {input:?}, got {err:?}"
            );
        }
    }

    // ── unterminated streams (WZ canvas shape) ─────────────────────
    //
    // WZ canvas blobs end without a BFINAL block or an adler32 trailer, so an
    // inflater never sees a stream terminator. Reproduced here with a
    // sync-flushed (never finished) stream, which has exactly that shape.

    /// zlib-headed stream carrying `payload` but never terminated.
    fn unterminated_zlib(payload: &[u8]) -> Vec<u8> {
        use flate2::{Compress, FlushCompress};

        let mut deflater = Compress::new(Compression::default(), true);
        let mut out = Vec::with_capacity(payload.len() + 128);
        deflater
            .compress_vec(payload, &mut out, FlushCompress::Sync)
            .unwrap();
        // No `FlushCompress::Finish` → no BFINAL block, no adler32 trailer.
        assert_eq!(deflater.total_in() as usize, payload.len());
        out
    }

    #[test]
    fn test_decompress_png_data_unterminated_zlib_canvas() {
        // A real 26×16 BGRA4444 face canvas: 832 = w·h·2 raw bytes, stored as
        // an unterminated stream. The decoded length must be exactly w·h·2 and
        // the pixels must survive the missing terminator.
        let (w, h) = (26u32, 16u32);
        let expected_len = WzPngFormat::Bgra4444.raw_data_size(w, h);
        assert_eq!(expected_len, (w * h * 2) as usize);

        let payload: Vec<u8> = (0..expected_len).map(|i| (i % 251) as u8).collect();
        let stream = unterminated_zlib(&payload);

        let raw = decompress_png_data(&stream, None).unwrap();
        assert_eq!(raw.len(), expected_len, "truncated stream lost pixel bytes");
        assert_eq!(raw, payload);

        // …and the canvas still decodes end-to-end to RGBA8888.
        let rgba = decode_pixels(&raw, w, h, WzPngFormat::Bgra4444).unwrap();
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }

    #[test]
    fn test_decompress_png_data_unterminated_raw_deflate() {
        use flate2::{Compress, FlushCompress};

        let payload: Vec<u8> = (0..4096).map(|i| (i % 97) as u8).collect();
        let mut deflater = Compress::new(Compression::default(), false); // raw deflate
        let mut stream = Vec::with_capacity(payload.len() + 128);
        deflater
            .compress_vec(&payload, &mut stream, FlushCompress::Sync)
            .unwrap();

        let raw = decompress_png_data(&stream, None).unwrap();
        assert_eq!(raw, payload);
    }

    #[test]
    fn test_decompress_png_data_unterminated_encrypted_blocks() {
        let payload: Vec<u8> = (0..1024).map(|i| (i % 211) as u8).collect();
        let zlib_data = unterminated_zlib(&payload);

        let mut compressed = Vec::new();
        compressed.extend_from_slice(&(zlib_data.len() as i32).to_le_bytes());
        compressed.extend_from_slice(&zlib_data);

        let key = vec![0u8; zlib_data.len()]; // zero key → XOR is a no-op
        let raw = decompress_png_data(&compressed, Some(&key)).unwrap();
        assert_eq!(raw, payload);
    }

    #[test]
    fn test_decompress_png_data_corrupt_still_errors() {
        // Tolerating a missing terminator must not swallow real data errors.
        let cases: Vec<(&str, Vec<u8>)> = vec![
            // zlib header + stored block whose NLEN is not !LEN
            (
                "invalid stored block lengths",
                vec![0x78, 0x9C, 0x00, 0x05, 0x00, 0x00, 0x00, 1, 2, 3, 4, 5],
            ),
            // raw deflate with the reserved block type 0b11
            ("reserved block type", vec![0xFF, 0xFF, 0x00, 0x00]),
            // zlib header, then nothing decodable at all
            ("empty after header", vec![0x78, 0x9C]),
        ];

        for (what, input) in cases {
            let err = decompress_png_data(&input, None).unwrap_err();
            assert!(
                matches!(err, WzError::DecompressionFailed(_)),
                "{what}: expected DecompressionFailed, got {err:?}"
            );
        }
    }

    #[test]
    fn test_decompress_png_data_encrypted_blocks() {
        // Build encrypted block format: [block_size:i32][encrypted_bytes:block_size]
        // With zero-key, XOR with key is a no-op, so we just need valid zlib after the 2-byte skip
        let original = b"test data for blocks";
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let zlib_data = encoder.finish().unwrap();

        // key = all zeros (BMS), so encrypted = plaintext.
        // Format: i32(block_size) + block_bytes
        let block_size = zlib_data.len() as i32;
        let mut compressed = Vec::new();
        compressed.extend_from_slice(&block_size.to_le_bytes());
        compressed.extend_from_slice(&zlib_data);

        let key = vec![0u8; zlib_data.len()]; // zero key
        let result = decompress_png_data(&compressed, Some(&key)).unwrap();
        assert_eq!(result, original);
    }
}
