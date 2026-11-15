//! A PNG writer for the screen of the 9h machine. It writes 8-bit RGB with stored deflate blocks,
//! so it needs no compression library. A screen of 1024 by 768 gives a file of about 2.3 MiB.

/// The CRC-32 of PNG (ISO 3309), over `data`, starting from `crc`.
fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(crc32(0, &kind), data);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// The zlib stream of `raw` in stored blocks.
fn stored(raw: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xffff).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        let n = b.len() as u16;
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(raw).to_be_bytes());
    z
}

/// The PNG file of an image of `width` by `height` pixels. `rgb` has 3 bytes for each pixel, row
/// by row.
#[must_use]
pub fn rgb(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let row = width as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * height as usize);
    for r in rgb.chunks(row).take(height as usize) {
        raw.push(0);
        raw.extend_from_slice(r);
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &ihdr);
    chunk(&mut out, *b"IDAT", &stored(&raw));
    chunk(&mut out, *b"IEND", &[]);
    out
}

#[cfg(test)]
mod tests {
    use super::{adler32, crc32, rgb, stored};

    #[test]
    fn the_checksums_have_the_known_values() {
        assert_eq!(crc32(0, b"123456789"), 0xcbf4_3926);
        assert_eq!(crc32(0, b"IEND"), 0xae42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }

    #[test]
    fn a_large_stream_has_more_than_one_block() {
        let raw = vec![7u8; 0x1_0000 + 10];
        let z = stored(&raw);
        assert_eq!(z.len(), 2 + 2 * 5 + raw.len() + 4);
        assert_eq!(z[2], 0, "the first block is not the last");
        assert_eq!(z[2 + 5 + 0xffff], 1, "the second block is the last");
    }

    #[test]
    fn a_png_has_the_signature_and_the_chunks() {
        let p = rgb(2, 1, &[255, 0, 0, 0, 0, 255]);
        assert!(p.starts_with(b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x02\0\0\0\x01\x08\x02"));
        assert!(p.ends_with(b"\0\0\0\0IEND\xae\x42\x60\x82"));
    }
}
