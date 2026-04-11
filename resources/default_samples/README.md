# Autokit default sample kit

These 16 one-shot drum samples ship embedded in the Autokit binary via
`include_bytes!` in `src/util/default_kit.rs`. They are loaded onto the pads
on first launch (no config, no persisted state) so a brand-new user has
something playable immediately — even before they point Autokit at their
own sample library.

Once a user scans their own folder, the normal scanner+dice flow takes over
and these defaults are replaced.

## Provenance & license

Rendered with [slammer](https://github.com/Hornfisk/slammer) — a sister
drum synthesizer project by the same author. Every file here is an
original synthesis of slammer and is distributed under the same GPL-3.0
terms as the rest of Autokit. No third-party commercial sample packs are
included.

## File list

| File              | Suggested pad role |
|-------------------|--------------------|
| bd_909.wav        | Kick — 909-style   |
| bd_clean.wav      | Kick — clean       |
| bd_drive.wav      | Kick — driven      |
| bd_hard.wav       | Kick — hard        |
| bd_hard2.wav      | Kick — hard alt    |
| bd_overdrive.wav  | Kick — overdriven  |
| bd_psy.wav        | Kick — psy         |
| bd_analogue.wav   | Kick — analogue    |
| sd_1.wav          | Snare              |
| sd_2.wav          | Snare alt          |
| hh_1.wav          | Hihat — closed     |
| hh_2.wav          | Hihat — open       |
| 808.wav           | 808 / bass         |
| tom.wav           | Tom — low          |
| tom2.wav          | Tom — mid          |
| hitom.wav         | Tom — high         |

Total size: ~2.9 MB uncompressed.
