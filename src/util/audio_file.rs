use std::io::Cursor;

use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode an in-memory audio blob (e.g. `include_bytes!`) to mono f32.
/// Used for the bundled default sample kit shipped inside the binary.
/// Resamples to `target_rate` if the file's native rate differs.
pub fn load_wav_mono_from_bytes(bytes: &'static [u8], label: &str, target_rate: f32) -> Result<Vec<f32>, String> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("wav");
    let (samples, native_rate) = decode_mono(mss, hint, label)?;
    resample_if_needed(samples, native_rate, target_rate, label)
}

/// Load an audio file to mono f32 samples, resampled to `target_rate`.
pub fn load_wav_mono(path: &str, target_rate: f32) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if path.ends_with(".wav") {
        hint.with_extension("wav");
    }

    let (samples, native_rate) = decode_mono(mss, hint, path)?;
    resample_if_needed(samples, native_rate, target_rate, path)
}

fn decode_mono(mss: MediaSourceStream, hint: Hint, path: &str) -> Result<(Vec<f32>, u32), String> {
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe {path}: {e}"))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| format!("no audio track in {path}"))?;

    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
    let native_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("decoder {path}: {e}"))?;

    let mut samples = Vec::new();

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(format!("packet read: {e}")),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder.decode(&packet).map_err(|e| format!("decode: {e}"))?;
        let spec = *decoded.spec();
        let duration = decoded.capacity();

        let mut buf = SampleBuffer::<f32>::new(duration as u64, spec);
        buf.copy_interleaved_ref(decoded);

        let interleaved = buf.samples();
        if channels == 1 {
            samples.extend_from_slice(interleaved);
        } else {
            // Downmix to mono by averaging channels
            for chunk in interleaved.chunks(channels) {
                let sum: f32 = chunk.iter().sum();
                samples.push(sum / channels as f32);
            }
        }
    }

    Ok((samples, native_rate))
}

/// Resample mono f32 audio from `native_rate` to `target_rate` using sinc
/// interpolation. Returns the input unchanged if rates already match.
fn resample_if_needed(samples: Vec<f32>, native_rate: u32, target_rate: f32, label: &str) -> Result<Vec<f32>, String> {
    let target = target_rate as u32;
    if native_rate == target || samples.is_empty() {
        return Ok(samples);
    }

    let ratio = target_rate as f64 / native_rate as f64;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };

    let chunk_size = 1024;
    let mut resampler = SincFixedIn::<f64>::new(
        ratio,
        2.0,  // max relative ratio (fixed ratio, so this is just headroom)
        params,
        chunk_size,
        1, // mono
    ).map_err(|e| format!("resampler init for {label}: {e}"))?;

    let mut output = Vec::with_capacity((samples.len() as f64 * ratio * 1.1) as usize);
    let mut pos = 0;

    while pos < samples.len() {
        let end = (pos + chunk_size).min(samples.len());
        let mut chunk: Vec<f64> = samples[pos..end].iter().map(|&s| s as f64).collect();

        // Pad the last chunk to chunk_size
        if chunk.len() < chunk_size {
            chunk.resize(chunk_size, 0.0);
        }

        let result = resampler.process(&[chunk], None)
            .map_err(|e| format!("resample {label}: {e}"))?;

        for &s in &result[0] {
            output.push(s as f32);
        }
        pos += chunk_size;
    }

    // Trim padding silence from the end (from the zero-padded last chunk)
    let expected_len = (samples.len() as f64 * ratio).ceil() as usize;
    output.truncate(expected_len);

    tracing::debug!(
        native_rate,
        target_rate = target,
        input_len = samples.len(),
        output_len = output.len(),
        "{label}: resampled"
    );

    Ok(output)
}
