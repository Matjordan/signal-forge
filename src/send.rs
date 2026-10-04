use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding { Text, Hex }
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineEnding { None, Cr, Lf, CrLf }

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[error("{0}")]
pub struct ParseError(pub String);

/// Fully validates before returning any bytes. Sending is a separate operation.
pub fn encode(input: &str, encoding: Encoding, escapes: bool, ending: LineEnding) -> Result<Vec<u8>, ParseError> {
    let mut output = match encoding {
        Encoding::Text if escapes => parse_escapes(input)?,
        Encoding::Text => input.as_bytes().to_vec(),
        Encoding::Hex => {
            let compact: String = input.chars().filter(|c| !c.is_ascii_whitespace()).collect();
            if compact.len() % 2 != 0 || !compact.is_ascii() { return Err(ParseError("Hex requires pairs of ASCII hex digits (example: 00 FF 0D 0A)".into())); }
            compact.as_bytes().chunks_exact(2).map(|pair| {
                let text = std::str::from_utf8(pair).expect("ASCII checked above");
                u8::from_str_radix(text, 16).map_err(|_| ParseError(format!("Invalid hex byte: {text}")))
            }).collect::<Result<Vec<_>, _>>()?
        }
    };
    output.extend_from_slice(match ending { LineEnding::None => b"", LineEnding::Cr => b"\r", LineEnding::Lf => b"\n", LineEnding::CrLf => b"\r\n" });
    Ok(output)
}

fn parse_escapes(input: &str) -> Result<Vec<u8>, ParseError> {
    let mut output = Vec::new();
    let mut chars = input.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            let mut buffer = [0;4];
            output.extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
            continue;
        }
        let byte = match chars.next() {
            Some('r') => b'\r', Some('n') => b'\n', Some('t') => b'\t', Some('0') => 0, Some('\\') => b'\\',
            Some('x') => {
                let high = chars.next().and_then(|c| c.to_digit(16));
                let low = chars.next().and_then(|c| c.to_digit(16));
                match (high, low) { (Some(h),Some(l)) => ((h << 4) | l) as u8, _ => return Err(ParseError("\\x must be followed by two hex digits".into())) }
            }
            Some(other) => return Err(ParseError(format!("Unknown escape: \\{other}"))),
            None => return Err(ParseError("Trailing backslash; use \\\\ for a literal backslash".into())),
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
        assert_eq!(encode("test\\r\\n", Encoding::Text, true, LineEnding::None).unwrap(), b"test\r\n");
        assert_eq!(encode("\\0\\xFF\\t\\\\", Encoding::Text, true, LineEnding::None).unwrap(), [0,255,9,92]);
        assert_eq!(encode("00 ff", Encoding::Hex, false, LineEnding::CrLf).unwrap(), [0,255,13,10]);
        assert_eq!(encode("λ", Encoding::Text, true, LineEnding::None).unwrap(), "λ".as_bytes());
        assert_eq!(encode("\\n", Encoding::Text, false, LineEnding::Lf).unwrap(), b"\\n\n");
    }
    #[test]
    fn invalid_input_is_rejected_as_a_whole() {
        for input in ["valid\\q", "valid\\", "valid\\x0", "valid\\xGG"] { assert!(encode(input,Encoding::Text,true,LineEnding::None).is_err()); }
        for input in ["0", "00 GG", "é", "0xFF"] { assert!(encode(input,Encoding::Hex,false,LineEnding::None).is_err()); }
    }
}
