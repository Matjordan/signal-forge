use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    Text,
    Hex,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding {
    None,
    Cr,
    Lf,
    CrLf,
}

/// XOR is computed over parsed payload bytes, before the configured line ending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChecksumOutput {
    Hex,
    StarHex,
    Raw,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    pub skip_first: bool,
    pub output: ChecksumOutput,
}
impl Default for Checksum {
    fn default() -> Self {
        Self {
            skip_first: false,
            output: ChecksumOutput::StarHex,
        }
    }
}
pub fn xor(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |sum, byte| sum ^ byte)
}
impl Checksum {
    pub fn value(self, payload: &[u8]) -> u8 {
        xor(if self.skip_first {
            payload.get(1..).unwrap_or_default()
        } else {
            payload
        })
    }
    /// Reusable transform for an already parsed payload, without its appended terminator.
    pub fn append(self, payload: &mut Vec<u8>) {
        let value = self.value(payload);
        match self.output {
            ChecksumOutput::Hex => payload.extend_from_slice(format!("{value:02X}").as_bytes()),
            ChecksumOutput::StarHex => {
                payload.extend_from_slice(format!("*{value:02X}").as_bytes())
            }
            ChecksumOutput::Raw => payload.push(value),
        }
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ParseError(pub String);

/// Fully validates before returning any bytes. Sending is a separate operation.
pub fn encode(
    input: &str,
    encoding: Encoding,
    escapes: bool,
    ending: LineEnding,
) -> Result<Vec<u8>, ParseError> {
    encode_with_checksum(input, encoding, escapes, ending, None)
}

/// Parse/validate the entire payload, inject its checksum, then append the line ending.
pub fn encode_with_checksum(
    input: &str,
    encoding: Encoding,
    escapes: bool,
    ending: LineEnding,
    checksum: Option<Checksum>,
) -> Result<Vec<u8>, ParseError> {
    let mut output = match encoding {
        Encoding::Text if escapes => parse_escapes(input)?,
        Encoding::Text => input.as_bytes().to_vec(),
        Encoding::Hex => {
            let compact: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            if compact.len() % 2 != 0 || !compact.is_ascii() {
                return Err(ParseError(
                    "Hex requires pairs of ASCII hex digits (example: 00 FF 0D 0A)".into(),
                ));
            }
            compact
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| {
                    let text = std::str::from_utf8(pair).expect("ASCII checked above");
                    u8::from_str_radix(text, 16)
                        .map_err(|_| ParseError(format!("Invalid hex byte: {text}")))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    if let Some(checksum) = checksum {
        checksum.append(&mut output);
    }
    output.extend_from_slice(match ending {
        LineEnding::None => b"",
        LineEnding::Cr => b"\r",
        LineEnding::Lf => b"\n",
        LineEnding::CrLf => b"\r\n",
    });
    Ok(output)
}

fn parse_escapes(input: &str) -> Result<Vec<u8>, ParseError> {
    let mut output = Vec::new();
    let mut chars = input.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut buffer = [0; 4];
            output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let byte = match chars.next() {
            Some('r') => b'\r',
            Some('n') => b'\n',
            Some('t') => b'\t',
            Some('0') => 0,
            Some('\\') => b'\\',
            Some('x') => {
                let high = chars.next().and_then(|c| c.to_digit(16));
                let low = chars.next().and_then(|c| c.to_digit(16));
                match (high, low) {
                    (Some(h), Some(l)) => ((h << 4) | l) as u8,
                    _ => return Err(ParseError("\\x must be followed by two hex digits".into())),
                }
            }
            Some(other) => return Err(ParseError(format!("Unknown escape: \\{other}"))),
            None => {
                return Err(ParseError(
                    "Trailing backslash; use \\\\ for a literal backslash".into(),
                ))
            }
        };
        output.push(byte);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_and_binary_inputs() {
        assert_eq!(
            encode("test\\r\\n", Encoding::Text, true, LineEnding::None).unwrap(),
            b"test\r\n"
        );
        assert_eq!(
            encode("\\0\\xFF\\t\\\\", Encoding::Text, true, LineEnding::None).unwrap(),
            [0, 255, 9, 92]
        );
        assert_eq!(
            encode("00 ff", Encoding::Hex, false, LineEnding::CrLf).unwrap(),
            [0, 255, 13, 10]
        );
        assert_eq!(
            encode("λ", Encoding::Text, true, LineEnding::None).unwrap(),
            "λ".as_bytes()
        );
        assert_eq!(
            encode("\\n", Encoding::Text, false, LineEnding::Lf).unwrap(),
            b"\\n\n"
        );
    }
    #[test]
    fn invalid_input_is_rejected_as_a_whole() {
        for input in ["valid\\q", "valid\\", "valid\\x0", "valid\\xGG"] {
            assert!(encode(input, Encoding::Text, true, LineEnding::None).is_err());
        }
        for input in ["0", "00 GG", "é", "0xFF"] {
            assert!(encode(input, Encoding::Hex, false, LineEnding::None).is_err());
        }
    }
}
