# Autokit

A drum machine plugin for Linux, inspired by XLN Audio XO and Algonaut Atlas 2.

Autokit scans your sample library, classifies oneshots by type (kick, snare, hihat, etc.) using DSP analysis, and plots them on a 2D map. Click to preview, assign to pads, sequence with a Digitakt-style step sequencer, and randomize kits with dice.

**Formats:** VST3, CLAP, Standalone  
**Platform:** Linux (x86_64)  
**License:** GPL-3.0-or-later

## Features

- **Sample analysis** — recursive scan of your sample folder, classifying ~1700 oneshots from spectral and temporal features. Results are cached for fast startup.
- **8-pad drum kit** — 2 kick, 1 snare, 1 hihat, 1 clap, 1 perc, 1 cymbal, 1 tom. Per-pad volume, pan, pitch, and decay.
- **2D sample map** — scatter plot of your entire library by spectral centroid (x) and decay time (y). Zoom, pan, hover to preview, click to assign. Shortcut mode for fast kit building.
- **Step sequencer** — 16-step, 8-track grid with per-step velocity, probability, pan/pitch p-locks, and conditional trigs (1:2, 1:4, Fill, etc.). 16 patterns, swing, FILL mode, DICE randomization.
- **Undo/redo** — 64-deep snapshot history covering kit and sequencer state.
- **Presets** — save/load JSON presets.
- **Standalone + plugin** — runs as a desktop app with internal transport, or syncs to host tempo in a DAW.

## Screenshots

*Coming soon*

## Building from source

### Prerequisites

**Rust toolchain** (stable, 1.75+):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**System dependencies** (Debian/Ubuntu):

```bash
sudo apt update
sudo apt install build-essential pkg-config libx11-dev libxcursor-dev \
    libxkbcommon-dev libgl1-mesa-dev libasound2-dev libxcb-render0-dev \
    libxcb-shape0-dev libxcb-xfixes0-dev libclang-dev
```

On Arch Linux: `sudo pacman -S base-devel libx11 libxcursor libxkbcommon mesa alsa-lib libxcb clang`

**Audio server:** PipeWire or JACK (PipeWire recommended, ships by default on most modern distros).

### Build

```bash
git clone https://github.com/YOUR_USER/Autokit.git
cd Autokit
cargo build --release
```

This produces:
- `target/release/libautokit.so` — shared library (VST3/CLAP)
- `target/release/autokit-standalone` — standalone binary

### Install

**VST3:**
```bash
mkdir -p ~/.vst3/autokit.vst3/Contents/x86_64-linux
cp target/release/libautokit.so ~/.vst3/autokit.vst3/Contents/x86_64-linux/autokit.so
```

**CLAP:**
```bash
mkdir -p ~/.clap
cp target/release/libautokit.so ~/.clap/autokit.clap
```

**Standalone** (run directly):
```bash
./target/release/autokit-standalone
```

### Verify installation

```bash
# Check the plugin has GUI code linked
strings ~/.vst3/autokit.vst3/Contents/x86_64-linux/autokit.so | grep -c egui
# Should output a number > 0
```

Rescan plugins in your DAW (Renoise: Preferences → VST/CLAP paths → Rescan). Autokit should appear as **REXIST / Autokit**.

## Sample library

Autokit scans `~/Music/Samples` on first launch. Place your oneshot samples (WAV, FLAC, OGG) there. The scan classifies files by spectral and temporal features, filtering out loops and long files. Results are cached at `~/.cache/autokit/library_cache.json`.

First scan of ~3800 files takes about 1 minute. Subsequent launches load from cache in under a second.

## Usage

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| Z X C V B N M , ; | Trigger pads 1-8 |
| Space | Play / Stop sequencer |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |

### Views

- **PADS** — vertical pad list with waveforms, per-pad volume/pan/pitch/decay knobs, dice and lock buttons
- **MAP** — 2D scatter plot of the sample library, click to preview and assign samples to pads
- **SEQ** — step sequencer grid with pattern bank, per-step parameters, conditional trigs

### Sequencer

- **Left-click** a cell to toggle a step on/off. Drag across cells to paint.
- **Right-click** an active step to select it for parameter editing (velocity, pan, pitch, probability, condition).
- **PLAY/STOP** button or Space bar to start/stop.
- **FILL** button (hold) activates Fill-conditioned steps.
- **DICE** randomizes the pattern (respects track locks).
- **Pattern bank** (01-16) along the top. Click to switch patterns (queues at next bar boundary).

## Architecture

```
src/
  lib.rs              Module structure, VST3+CLAP export macros
  main.rs             Standalone binary entry point
  plugin.rs           Plugin trait impl, audio processing, MIDI, sequencer wiring
  logging.rs          Ring-buffer tracing to ~/.local/share/autokit/autokit.log
  engine/
    kit.rs            DrumKit (8 pads), sample categories, dice/lock, snapshots
    sampler.rs        VoicePool (32 voices), constant-power pan, pitch shifting
    sequencer.rs      Sequencer, PatternBank (16 patterns), conditional trigs
  analysis/
    scanner.rs        Recursive file walker, oneshot filter
    features.rs       Spectral centroid/flatness (FFT), attack/decay, classification
    library.rs        SampleLibrary, kit generation, random selection
    cache.rs          Persistent JSON cache with mtime validation
  ui/
    editor.rs         egui editor, view modes, keyboard triggers, GUI actions
    theme.rs          Color palette, fonts, category colors
    toolbar.rs        Top bar: logo, view tabs, undo/redo, dice, presets, volume
    pad_row.rs        Pad strip: collapsed/expanded views, waveforms, knobs
    sample_map.rs     2D scatter plot, zoom/pan, hover preview, assignment
    sequencer_ui.rs   Step grid, pattern bar, param bar, play controls
    knob.rs           Circular knob widget
    waveform.rs       Waveform polyline renderer
    state.rs          SharedState, WaveformSummary, ScanStatus
  util/
    audio_file.rs     WAV/FLAC/OGG loading via symphonia
    history.rs        64-deep undo/redo snapshots
    preset.rs         JSON preset save/load
```

## Known limitations

- Sample library root is hard-coded to `~/Music/Samples`. Configurable path is planned.
- The standalone sequencer uses internal timing only. In a DAW, the sequencer responds to the PLAY button in the plugin UI, not directly to host transport.
- Window cannot be dynamically resized (egui-baseview limitation).

## Dependencies

Built with [nih-plug](https://github.com/robbert-vdh/nih-plug) (plugin framework), [egui](https://github.com/emilk/egui) (GUI), [symphonia](https://github.com/pdeljanov/symphonia) (audio decoding), and [realfft](https://crates.io/crates/realfft) (spectral analysis). See `Cargo.toml` for the full list.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE) for details.
