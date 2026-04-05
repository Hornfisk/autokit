/// Ring buffer size — 32 entries covers worst case of 8 pads firing per step
/// with several steps' worth of overlap.
const BUFFER_SIZE: usize = 32;

/// Maximum samples between outgoing and incoming note to count as echo.
/// ~50ms at 48kHz — covers host round-trip across multiple buffer sizes.
const ECHO_WINDOW: u64 = 2400;

/// Consecutive echo matches before activating suppression mode.
const SUPPRESS_THRESHOLD: u32 = 4;

/// Samples with no echoes before clearing suppression (~2s at 48kHz).
const RECOVERY_SAMPLES: u64 = 96000;

/// Detects when incoming MIDI notes are echoes of recent sequencer output.
///
/// Tracks outgoing notes in a fixed-size ring buffer. When an incoming note
/// matches a recent outgoing note within the timing window, it's flagged as
/// an echo and should be suppressed to prevent doubled playback.
pub struct EchoDetector {
    buffer: [(u8, u64); BUFFER_SIZE],
    write_pos: usize,
    len: usize,
    sample_clock: u64,
    consecutive_echoes: u32,
    suppressing: bool,
    last_echo_clock: u64,
}

impl EchoDetector {
    pub fn new() -> Self {
        Self {
            buffer: [(0, 0); BUFFER_SIZE],
            write_pos: 0,
            len: 0,
            sample_clock: 0,
            consecutive_echoes: 0,
            suppressing: false,
            last_echo_clock: 0,
        }
    }

    /// Advance the sample clock and check recovery timeout.
    pub fn tick(&mut self, buffer_len: usize) {
        self.sample_clock += buffer_len as u64;

        if self.suppressing
            && self.sample_clock.saturating_sub(self.last_echo_clock) > RECOVERY_SAMPLES
        {
            self.suppressing = false;
            self.consecutive_echoes = 0;
        }
    }

    /// Record an outgoing sequencer note for future echo matching.
    pub fn record(&mut self, note: u8) {
        self.buffer[self.write_pos] = (note, self.sample_clock);
        self.write_pos = (self.write_pos + 1) % BUFFER_SIZE;
        if self.len < BUFFER_SIZE {
            self.len += 1;
        }
    }

    /// Check if an incoming note is an echo of a recent outgoing note.
    /// Returns `true` if it's an echo (caller should suppress the trigger).
    pub fn check(&mut self, note: u8) -> bool {
        let mut found = None;

        // Scan buffer for a matching note within the timing window
        let start = if self.len < BUFFER_SIZE {
            0
        } else {
            self.write_pos
        };

        for i in 0..self.len {
            let idx = (start + i) % BUFFER_SIZE;
            let (buf_note, buf_clock) = self.buffer[idx];
            if buf_note == note && self.sample_clock.saturating_sub(buf_clock) <= ECHO_WINDOW {
                found = Some(idx);
                break;
            }
        }

        if let Some(idx) = found {
            // Consume the match — set note to 0xFF (invalid MIDI) so it can't match again
            self.buffer[idx].0 = 0xFF;
            self.consecutive_echoes += 1;
            self.last_echo_clock = self.sample_clock;
            if self.consecutive_echoes >= SUPPRESS_THRESHOLD {
                self.suppressing = true;
            }
            true
        } else {
            self.consecutive_echoes = 0;
            false
        }
    }

    /// Whether the detector is in active suppression mode (for UI "EXT" indicator).
    pub fn is_suppressing(&self) -> bool {
        self.suppressing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_note_within_window_is_echo() {
        let mut det = EchoDetector::new();
        det.record(36);
        // Simulate a few samples passing (within window)
        det.tick(100);
        assert!(det.check(36), "same note within window should be echo");
    }

    #[test]
    fn same_note_outside_window_is_not_echo() {
        let mut det = EchoDetector::new();
        det.record(36);
        det.tick(ECHO_WINDOW as usize + 100);
        assert!(!det.check(36), "same note outside window should not be echo");
    }

    #[test]
    fn different_note_is_not_echo() {
        let mut det = EchoDetector::new();
        det.record(36);
        det.tick(100);
        assert!(!det.check(38), "different note should not be echo");
    }

    #[test]
    fn match_is_consumed_once() {
        let mut det = EchoDetector::new();
        det.record(36);
        det.tick(100);
        assert!(det.check(36));
        assert!(!det.check(36), "second check should not match consumed entry");
    }

    #[test]
    fn suppressing_activates_after_threshold() {
        let mut det = EchoDetector::new();
        assert!(!det.is_suppressing());

        for i in 0..SUPPRESS_THRESHOLD {
            det.record(36 + (i as u8 % 4));
            det.tick(50);
            det.check(36 + (i as u8 % 4));
        }

        assert!(det.is_suppressing(), "should suppress after {} consecutive echoes", SUPPRESS_THRESHOLD);
    }

    #[test]
    fn non_echo_resets_consecutive_count() {
        let mut det = EchoDetector::new();

        // Build up 3 echoes (below threshold)
        for _ in 0..3 {
            det.record(36);
            det.tick(50);
            det.check(36);
        }
        assert!(!det.is_suppressing());

        // Non-echo resets the count
        det.check(42);
        assert_eq!(det.consecutive_echoes, 0);

        // Need full threshold again
        for _ in 0..3 {
            det.record(36);
            det.tick(50);
            det.check(36);
        }
        assert!(!det.is_suppressing(), "should not suppress — count was reset");
    }

    #[test]
    fn recovery_clears_suppressing() {
        let mut det = EchoDetector::new();

        // Activate suppression
        for i in 0..SUPPRESS_THRESHOLD {
            det.record(36 + (i as u8 % 4));
            det.tick(50);
            det.check(36 + (i as u8 % 4));
        }
        assert!(det.is_suppressing());

        // Wait for recovery
        det.tick(RECOVERY_SAMPLES as usize + 1);
        assert!(!det.is_suppressing(), "should clear after recovery timeout");
    }

    #[test]
    fn ring_buffer_wraps_without_panic() {
        let mut det = EchoDetector::new();
        // Overflow the buffer
        for i in 0..(BUFFER_SIZE * 2) {
            det.record((i % 128) as u8);
        }
        det.tick(100);
        // Should still work
        det.record(60);
        det.tick(50);
        assert!(det.check(60));
    }
}
