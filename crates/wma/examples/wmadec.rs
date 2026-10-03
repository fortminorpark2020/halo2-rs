//! Decode a WMA 2 stream in a WAV file to raw 16-bit samples:
//! `wmadec in.wav out.raw` (for checking against other decoders).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let [_, input, output] = &args[..] else {
        return Err("usage: wmadec in.wav out.raw".into());
    };
    let data = std::fs::read(input)?;
    let (mut format, mut samples) = (None, None);
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes(data[pos + 4..pos + 8].try_into()?) as usize;
        let body = &data[pos + 8..(pos + 8 + len).min(data.len())];
        match id {
            b"fmt " => format = Some(wma::Format::parse(body)?),
            b"data" => samples = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let format = format.ok_or("no fmt chunk")?;
    let pcm = wma::decode(&format, samples.ok_or("no data chunk")?)?;
    let bytes: Vec<u8> = pcm
        .iter()
        .flat_map(|&s| {
            ((s * 32768.0).round_ties_even().clamp(-32768.0, 32767.0) as i16).to_le_bytes()
        })
        .collect();
    std::fs::write(output, bytes)?;
    println!("{:?}: {} samples", format, pcm.len());
    Ok(())
}
