//! The frame JPEG encoders: turbojpeg by default, chosen by spike S2, and the
//! pure-Rust jpeg-encoder as the fallback feature. Both take packed BGRA and
//! write baseline JPEG with 4:2:0 chroma and standard Huffman tables.

use crate::error::Result;

/// A JPEG encoder at a fixed quality.
pub struct Encoder {
    _private: (),
}

impl Encoder {
    /// The preferred compiled-in encoder: turbojpeg, then jpeg-encoder when
    /// turbojpeg is not compiled in or cannot start.
    pub fn new(quality: u8) -> Result<Encoder> {
        let _ = quality;
        unimplemented!("task 6.2: JPEG encoding")
    }

    /// "turbojpeg" or "jpeg-encoder".
    pub fn name(&self) -> &'static str {
        unimplemented!("task 6.2: JPEG encoding")
    }

    /// Encodes packed BGRA `pixels` of `width` x `height`.
    pub fn encode(&mut self, pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
        let _ = (pixels, width, height);
        unimplemented!("task 6.2: JPEG encoding")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a baseline JPEG's SOF0 dimensions.
    fn sof_dimensions(jpeg: &[u8]) -> Option<(usize, usize)> {
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "SOI marker");
        let mut at = 2;
        while at + 9 <= jpeg.len() {
            let marker = jpeg[at + 1];
            let length = usize::from(u16::from_be_bytes([jpeg[at + 2], jpeg[at + 3]]));
            if marker == 0xC0 {
                let height = u16::from_be_bytes([jpeg[at + 5], jpeg[at + 6]]);
                let width = u16::from_be_bytes([jpeg[at + 7], jpeg[at + 8]]);
                return Some((usize::from(width), usize::from(height)));
            }
            at += 2 + length;
        }
        None
    }

    #[test]
    fn the_encoder_writes_a_baseline_jpeg_of_the_input_size() {
        let (width, height) = (46, 100);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| [(i % 256) as u8, 128, 64, 255])
            .collect();
        let mut encoder = Encoder::new(80).expect("an encoder is compiled in");
        let jpeg = encoder.encode(&pixels, width, height).expect("it encodes");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "EOI marker");
        assert_eq!(sof_dimensions(&jpeg), Some((width, height)));
        let expected = if cfg!(feature = "turbojpeg") {
            "turbojpeg"
        } else {
            "jpeg-encoder"
        };
        assert_eq!(encoder.name(), expected);
    }
}
