# wma

A Windows Media Audio 2 decoder: a Rust port of FFmpeg's WMA decoder
(`libavcodec/wma.c`, `wmadec.c`), with the bitstream tables in `src/tables.rs`
generated from FFmpeg's `wmadata.h` and `aactab.c`.

Because it is derived from FFmpeg, this crate is licensed under the GNU Lesser
General Public License, version 2.1 or later (see `LICENSE`), unlike the rest
of the repository.

Halo 2 PC stores its announcer and dialogue as WMA 2 (stereo, 44.1 kHz,
96 kbit/s). Decoding every one of those lines gives the same samples as
FFmpeg 6.1, to within one 16-bit step, as do FFmpeg-encoded streams at
8 to 48 kHz, mono and stereo, 24 to 128 kbit/s.

Not supported: WMA 1, and WMA 2 streams whose exponents are coded as line
spectral pairs.

`cargo run --example wmadec in.wav out.raw` decodes a WMA stream held in a
WAV file to raw 16-bit samples, for comparing with other decoders.
