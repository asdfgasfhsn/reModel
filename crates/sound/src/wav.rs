//! Minimal RIFF/WAVE writer for 16-bit PCM.

/// Encode interleaved 16-bit samples as a WAV file.
pub fn encode(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let block = channels as u32 * 2;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block).to_le_bytes());
    out.extend_from_slice(&(block as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn header() {
        let w = super::encode(44100, 2, &[1, -1]);
        assert_eq!(w.len(), 48);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(w[28..32].try_into().unwrap()), 44100 * 4);
        assert_eq!(&w[44..], &[1, 0, 0xff, 0xff]);
    }
}
