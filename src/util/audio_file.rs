use std::io::Cursor;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decode an in-memory audio blob (e.g. `include_bytes!`) to mono f32.
/// Used for the bundled default sample kit shipped inside the binary.
pub fn load_wav_mono_from_bytes(bytes: &'static [u8], label: &str) -> Result<Vec<f32>, String> {
    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("wav");
    decode_mono(mss, hint, label)
}

/// Load an audio file to mono f32 samples at native sample rate.
/// No resampling — that comes in Phase 3.
pub fn load_wav_mono(path: &str) -> Result<Vec<f32>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if path.ends_with(".wav") {
        hint.with_extension("wav");
    }

    decode_mono(mss, hint, path)
}

fn decode_mono(mss: MediaSourceStream, hint: Hint, path: &str) -> Result<Vec<f32>, String> {
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("probe {path}: {e}"))?;

    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| format!("no audio track in {path}"))?;

    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);
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

    Ok(samples)
}
