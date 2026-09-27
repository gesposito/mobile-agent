//! The frame JPEG encoders: turbojpeg by default, chosen by spike S2 for
//! being 2.7 to 3.1 times faster, and the pure-Rust jpeg-encoder as the
//! fallback feature. Both take packed BGRA and write baseline JPEG with 4:2:0
//! chroma and standard Huffman tables.

use crate::error::{Error, Result};

#[cfg(not(any(feature = "turbojpeg", feature = "jpeg-encoder")))]
compile_error!("crates/simulator needs a JPEG encoder: enable `turbojpeg` or `jpeg-encoder`");

/// A JPEG encoder at a fixed quality.
pub struct Encoder {
    backend: Backend,
}

enum Backend {
    #[cfg(feature = "turbojpeg")]
    Turbo(turbojpeg::Compressor),
    #[cfg(feature = "jpeg-encoder")]
    Pure { quality: u8 },
}

impl Encoder {
    /// The preferred compiled-in encoder: turbojpeg, then jpeg-encoder when
    /// turbojpeg is not compiled in or cannot start. `quality` is clamped to
    /// 1 to 100.
    pub fn new(quality: u8) -> Result<Encoder> {
        Ok(Encoder {
            backend: Backend::start(quality.clamp(1, 100))?,
        })
    }

    /// "turbojpeg" or "jpeg-encoder".
    pub fn name(&self) -> &'static str {
        match self.backend {
            #[cfg(feature = "turbojpeg")]
            Backend::Turbo(_) => "turbojpeg",
            #[cfg(feature = "jpeg-encoder")]
            Backend::Pure { .. } => "jpeg-encoder",
        }
    }

    /// Encodes packed BGRA `pixels` of `width` x `height`.
    pub fn encode(&mut self, pixels: &[u8], width: usize, height: usize) -> Result<Vec<u8>> {
        match &mut self.backend {
            #[cfg(feature = "turbojpeg")]
            Backend::Turbo(compressor) => compressor
                .compress_to_vec(turbojpeg::Image {
                    pixels,
                    width,
                    pitch: width * 4,
                    height,
                    format: turbojpeg::PixelFormat::BGRA,
                })
                .map_err(|error| failed("turbojpeg", error)),
            #[cfg(feature = "jpeg-encoder")]
            Backend::Pure { quality } => {
                let (Ok(w), Ok(h)) = (u16::try_from(width), u16::try_from(height)) else {
                    return Err(failed(
                        "jpeg-encoder",
                        format!("{width}x{height} exceeds 65535 pixels"),
                    ));
                };
                let mut out = Vec::with_capacity(width * height / 4);
                let mut encoder = jpeg_encoder::Encoder::new(&mut out, *quality);
                encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_2_0);
                encoder.set_optimized_huffman_tables(false);
                encoder
                    .encode(pixels, w, h, jpeg_encoder::ColorType::Bgra)
                    .map_err(|error| failed("jpeg-encoder", error))?;
                Ok(out)
            }
        }
    }
}

impl Backend {
    #[cfg(all(feature = "turbojpeg", feature = "jpeg-encoder"))]
    fn start(quality: u8) -> Result<Backend> {
        Ok(turbo(quality)
            .map(Backend::Turbo)
            .unwrap_or(Backend::Pure { quality }))
    }

    #[cfg(all(feature = "turbojpeg", not(feature = "jpeg-encoder")))]
    fn start(quality: u8) -> Result<Backend> {
        turbo(quality).map(Backend::Turbo)
    }

    #[cfg(all(not(feature = "turbojpeg"), feature = "jpeg-encoder"))]
    fn start(quality: u8) -> Result<Backend> {
        Ok(Backend::Pure { quality })
    }
}

#[cfg(feature = "turbojpeg")]
fn turbo(quality: u8) -> Result<turbojpeg::Compressor> {
    let fail = |error: turbojpeg::Error| failed("turbojpeg", error);
    let mut compressor = turbojpeg::Compressor::new().map_err(fail)?;
    compressor.set_quality(i32::from(quality)).map_err(fail)?;
    compressor
        .set_subsamp(turbojpeg::Subsamp::Sub2x2)
        .map_err(fail)?;
    compressor.set_optimize(false).map_err(fail)?;
    Ok(compressor)
}

fn failed(encoder: &str, error: impl std::fmt::Display) -> Error {
    Error::Frame {
        reason: format!("{encoder}: {error}"),
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
