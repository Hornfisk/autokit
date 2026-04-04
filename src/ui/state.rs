use std::sync::Arc;

use crate::analysis::library::SampleLibrary;
use crate::engine::kit::{DrumKit, NUM_PADS};
use crate::engine::sequencer::PatternBank;
use crate::util::history::History;

/// Scan progress for the toolbar display.
#[derive(Clone, Debug)]
pub enum ScanStatus {
    Scanning,
    Ready { total: usize },
}

/// Pre-computed waveform display data for one pad.
/// Stores min/max pairs downsampled to `points` columns.
#[derive(Clone)]
pub struct WaveformSummary {
    /// (min, max) amplitude pairs, one per display column.
    pub points: Vec<[f32; 2]>,
}

impl WaveformSummary {
    /// Downsample raw sample data to `num_points` min/max pairs.
    pub fn from_samples(samples: &[f32], num_points: usize) -> Self {
        if samples.is_empty() || num_points == 0 {
            return Self { points: vec![] };
        }

        let chunk_size = samples.len() / num_points;
        if chunk_size == 0 {
            // Fewer samples than points — one point per sample
            return Self {
                points: samples.iter().map(|&s| [s, s]).collect(),
            };
        }

        let points = (0..num_points)
            .map(|i| {
                let start = i * chunk_size;
                let end = ((i + 1) * chunk_size).min(samples.len());
                let chunk = &samples[start..end];
                let min = chunk.iter().copied().fold(f32::INFINITY, f32::min);
                let max = chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                [min, max]
            })
            .collect();

        Self { points }
    }
}

/// State shared between the audio thread and GUI thread.
/// Locked via `parking_lot::Mutex` — locks must be brief.
pub struct SharedState {
    pub kit: DrumKit,
    pub library: Option<SampleLibrary>,
    pub history: History,
    pub scan_status: ScanStatus,
    /// Pre-computed waveform summaries, one per pad. Recomputed on sample change.
    pub waveforms: [Option<WaveformSummary>; NUM_PADS],
    /// Sample data to preview (set by GUI, consumed by audio thread).
    pub preview_sample: Option<Arc<Vec<f32>>>,
    /// Sequencer pattern data — edited by GUI, read by audio thread.
    pub pattern_bank: PatternBank,
    /// Pattern clipboard for copy/paste.
    pub pattern_clipboard: Option<crate::engine::sequencer::Pattern>,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            kit: DrumKit::new(),
            library: None,
            history: History::new(),
            scan_status: ScanStatus::Scanning,
            waveforms: Default::default(),
            preview_sample: None,
            pattern_bank: PatternBank::new(),
            pattern_clipboard: None,
        }
    }

    /// Recompute waveform summary for a single pad.
    pub fn update_waveform(&mut self, pad_index: usize, num_points: usize) {
        if pad_index >= self.kit.pads.len() {
            return;
        }
        self.waveforms[pad_index] = self.kit.pads[pad_index]
            .sample
            .as_ref()
            .map(|s| WaveformSummary::from_samples(s, num_points));
    }

    /// Recompute waveform summaries for all pads.
    pub fn update_all_waveforms(&mut self, num_points: usize) {
        for i in 0..NUM_PADS {
            self.update_waveform(i, num_points);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_summary_downsamples_correctly() {
        // 200 samples → 10 points = 20 samples per chunk
        let samples: Vec<f32> = (0..200).map(|i| (i as f32 / 200.0) * 2.0 - 1.0).collect();
        let summary = WaveformSummary::from_samples(&samples, 10);
        assert_eq!(summary.points.len(), 10);
        // First chunk: samples 0..20, values -1.0 to ~-0.8
        assert!(summary.points[0][0] < summary.points[0][1]); // min < max
    }

    #[test]
    fn waveform_summary_empty_input() {
        let summary = WaveformSummary::from_samples(&[], 10);
        assert!(summary.points.is_empty());
    }

    #[test]
    fn waveform_summary_fewer_samples_than_points() {
        let samples = vec![0.5, -0.3, 0.8];
        let summary = WaveformSummary::from_samples(&samples, 10);
        assert_eq!(summary.points.len(), 3);
    }
}
