use std::io::{BufWriter, Write};

use js_sys::Uint8Array;
use jxl_oxide::{color::RenderingIntent, JxlImage, PixelFormat};
use thiserror::Error;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn init() {
	console_error_panic_hook::set_once();
	tracing_wasm::set_as_global_default();
}

struct JsWrite<'a> {
	f: &'a js_sys::Function,
}

impl<'a> Write for JsWrite<'a> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		tracing::trace!(len = buf.len(), "JS write");
		match self.f.call1(&JsValue::null(), &Uint8Array::from(buf)) {
			Ok(_) => Ok(buf.len()),
			Err(e) => Err(std::io::Error::new(std::io::ErrorKind::Other, format!("JS error: {e:?}"))),
		}
	}

	fn flush(&mut self) -> std::io::Result<()> {
		Ok(())
	}
}

#[derive(Error, Debug)]
pub enum TranscodeError {
	#[error("JXL error: {0}")]
	Jxl(#[from] Box<dyn std::error::Error + Send + Sync + 'static>),
	#[error("PNG error: {0}")]
	Png(#[from] png::EncodingError),
	#[error("IO error: {0}")]
	Io(#[from] std::io::Error),
	#[error("Need more data")]
	NeedMoreData,
	#[error("Unsupported pixel format")]
	UnsupportedPixelFormat,
	#[error("Unsupported ICC profile")]
	UnsupportedIcc,
}

impl From<TranscodeError> for JsValue {
	fn from(val: TranscodeError) -> Self {
		format!("{val:#}").into()
	}
}

#[wasm_bindgen]
pub fn transcode(data: &[u8], write: &js_sys::Function) -> Result<(), TranscodeError> {
	let mut image = JxlImage::from_reader(data)?;
	let image_size = &image.image_header().size;
	let image_meta = &image.image_header().metadata;
	tracing::info!("Image dimension: {}x{}", image_size.width, image_size.height);
	tracing::debug!(colour_encoding = format_args!("{:?}", image_meta.colour_encoding));

	let (width, height) = (image_size.width, image_size.height);

	let mut keyframes = Vec::new();
	let mut renderer = image.renderer();
	loop {
		let result = renderer.render_next_frame()?;
		match result {
			jxl_oxide::RenderResult::Done(frame) => keyframes.push(frame),
			jxl_oxide::RenderResult::NeedMoreData => return Err(TranscodeError::NeedMoreData),
			jxl_oxide::RenderResult::NoMoreFrames => break,
		}
	}

	// Color encoding information
	let pixfmt = renderer.pixel_format();
	let source_icc = renderer.rendered_icc();
	let embedded_icc = image.embedded_icc();
	let metadata = &image.image_header().metadata;
	let colour_encoding = &metadata.colour_encoding;
	let cicp = colour_encoding.cicp();

	let (width, height, _, _) = metadata.apply_orientation(width, height, 0, 0, false);
	let writer = JsWrite { f: write };
	let writer = BufWriter::with_capacity(64 * 1024, writer);
	let chunk_size = writer.capacity();
	let mut encoder = png::Encoder::new(writer, width, height);

	let color_type = match pixfmt {
		PixelFormat::Gray => png::ColorType::Grayscale,
		PixelFormat::Graya => png::ColorType::GrayscaleAlpha,
		PixelFormat::Rgb => png::ColorType::Rgb,
		PixelFormat::Rgba => png::ColorType::Rgba,
		_ => return Err(TranscodeError::UnsupportedPixelFormat),
	};
	encoder.set_color(color_type);
	encoder.set_compression(png::Compression::Fast);

	let sixteen_bits = metadata.bit_depth.bits_per_sample() > 8;
	if sixteen_bits {
		encoder.set_depth(png::BitDepth::Sixteen);
	} else {
		encoder.set_depth(png::BitDepth::Eight);
	}

	if let Some(animation) = &metadata.animation {
		let num_plays = animation.num_loops;
		encoder.set_animated(keyframes.len() as u32, num_plays)?;
	}

	let icc_cicp = if let Some(icc) = embedded_icc {
		if metadata.xyb_encoded {
			return Err(TranscodeError::UnsupportedIcc);
		} else {
			Some((icc, None))
		}
	} else if colour_encoding.is_srgb() {
		encoder.set_srgb(match colour_encoding.rendering_intent {
			RenderingIntent::Perceptual => png::SrgbRenderingIntent::Perceptual,
			RenderingIntent::Relative => png::SrgbRenderingIntent::RelativeColorimetric,
			RenderingIntent::Saturation => png::SrgbRenderingIntent::Saturation,
			RenderingIntent::Absolute => png::SrgbRenderingIntent::AbsoluteColorimetric,
		});

		None
	} else {
		// TODO: emit gAMA and cHRM
		Some((&*source_icc, cicp))
	};
	encoder.validate_sequence(true);

	let mut writer = encoder.write_header()?;

	if let Some((icc, cicp)) = &icc_cicp {
		tracing::debug!("Embedding ICC profile");
		let compressed_icc = miniz_oxide::deflate::compress_to_vec_zlib(icc, 7);
		let mut iccp_chunk_data = vec![b'0', 0, 0];
		iccp_chunk_data.extend(compressed_icc);
		writer.write_chunk(png::chunk::iCCP, &iccp_chunk_data)?;

		if let Some(cicp) = *cicp {
			tracing::debug!(cicp = format_args!("{:?}", cicp), "Writing cICP chunk");
			writer.write_chunk(png::chunk::ChunkType([b'c', b'I', b'C', b'P']), &cicp)?;
		}
	}

	tracing::debug!("Writing image data");
	let mut writer = writer.stream_writer_with_size(chunk_size - 12)?;

	for keyframe in keyframes {
		if let Some(animation) = &metadata.animation {
			let duration = keyframe.duration();
			let numer = animation.tps_denominator * duration;
			let denom = animation.tps_numerator;
			let (numer, denom) = if numer >= 0x10000 || denom >= 0x10000 {
				if duration == 0xffffffff {
					tracing::warn!(numer, denom, "Writing multi-page image in APNG");
				} else {
					tracing::warn!(numer, denom, "Frame duration is not representable in APNG");
				}
				let duration = (numer as f32 / denom as f32) * 65535.0;
				(duration as u16, 0xffffu16)
			} else {
				(numer as u16, denom as u16)
			};
			writer.set_frame_delay(numer, denom)?;
		}

		let fb = keyframe.image();

		let mut buf = vec![0u8; 64 * 1024];

		if sixteen_bits {
			tracing::debug!("16 Bit");

			for win in fb.buf().chunks(buf.len() / 2) {
				for (b, s) in buf.chunks_exact_mut(2).zip(win) {
					let w = (*s * 65535.0 + 0.5).clamp(0.0, 65535.0) as u16;
					[b[0], b[1]] = w.to_be_bytes();
				}

				writer.write_all(&buf[..win.len() * 2])?;
			}
		} else {
			tracing::debug!("8 Bit");

			for win in fb.buf().chunks(buf.len()) {
				for (b, s) in buf.iter_mut().zip(win) {
					*b = (*s * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
				}

				writer.write_all(&buf[..win.len()])?;
			}
		}
	}

	writer.finish()?;

	Ok(())
}
