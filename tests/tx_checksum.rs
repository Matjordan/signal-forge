use signal_forge::send::{self, Checksum, ChecksumOutput as Output, Encoding, LineEnding};
fn encode(input: &str, skip: bool, output: Output, ending: LineEnding) -> Vec<u8> {
    send::encode_with_checksum(
        input,
        Encoding::Text,
        true,
        ending,
        Some(Checksum {
            skip_first: skip,
            output,
        }),
    )
    .unwrap()
}
#[test]
fn proprietary_payload_formats_and_skip_preserve_the_first_byte() {
    assert_eq!(
        encode("#0,MED", false, Output::Hex, LineEnding::None),
        b"#0,MED73"
    );
    assert_eq!(
        encode("#0,MED", false, Output::StarHex, LineEnding::CrLf),
        b"#0,MED*73\r\n"
    );
    assert_eq!(
        encode("#0,MED", true, Output::StarHex, LineEnding::CrLf),
        b"#0,MED*50\r\n"
    );
    assert_eq!(
        encode("#0,MED", true, Output::Raw, LineEnding::None),
        b"#0,MED\x50"
    );
    assert_eq!(
        encode("$ABC", true, Output::StarHex, LineEnding::CrLf),
        b"$ABC*40\r\n"
    );
}
#[test]
fn parsing_precedes_checksum_and_only_configured_terminators_are_excluded() {
    assert_eq!(
        encode("\\0\\xFF\\r\\n", false, Output::StarHex, LineEnding::CrLf),
        b"\0\xff\r\n*F8\r\n"
    );
    let checksum = Some(Checksum {
        skip_first: true,
        output: Output::Raw,
    });
    assert_eq!(
        send::encode_with_checksum("23 00 FF", Encoding::Hex, false, LineEnding::Lf, checksum)
            .unwrap(),
        [0x23, 0, 255, 255, 10]
    );
    assert_eq!(
        encode("λ", true, Output::Hex, LineEnding::None),
        [0xce, 0xbb, b'B', b'B']
    );
    for invalid in ["valid\\q", "valid\\x", "valid\\xGG"] {
        assert!(send::encode_with_checksum(
            invalid,
            Encoding::Text,
            true,
            LineEnding::None,
            checksum
        )
        .is_err());
    }
    assert!(
        send::encode_with_checksum("FF G0", Encoding::Hex, false, LineEnding::None, checksum)
            .is_err()
    );
}
#[test]
fn zero_empty_and_single_byte_are_predictable_and_off_is_unchanged() {
    for skip in [false, true] {
        assert_eq!(encode("", skip, Output::Hex, LineEnding::None), b"00");
        assert_eq!(encode("", skip, Output::Raw, LineEnding::Cr), [0, 13]);
    }
    assert_eq!(
        encode("A", true, Output::StarHex, LineEnding::None),
        b"A*00"
    );
    assert_eq!(encode("A", false, Output::Hex, LineEnding::None), b"A41");
    assert_eq!(
        encode("\\x01", false, Output::Hex, LineEnding::Lf),
        b"\x0101\n"
    );
    for (input, encoding, escapes) in [
        ("#0,MED", Encoding::Text, false),
        ("\\r\\xFF", Encoding::Text, true),
        ("00 FF", Encoding::Hex, false),
        ("", Encoding::Text, true),
    ] {
        for ending in [
            LineEnding::None,
            LineEnding::Cr,
            LineEnding::Lf,
            LineEnding::CrLf,
        ] {
            assert_eq!(
                send::encode_with_checksum(input, encoding, escapes, ending, None),
                send::encode(input, encoding, escapes, ending)
            );
        }
    }
}
#[test]
fn old_presets_default_off_and_new_settings_round_trip_with_wire_bytes() {
    use signal_forge::presets::Preset;
    let mut preset = Preset {
        payload: "#0,MED".into(),
        ending: LineEnding::CrLf,
        ..Default::default()
    };
    let mut json = serde_json::to_value(&preset).unwrap();
    json.as_object_mut().unwrap().remove("checksum");
    let legacy: Preset = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.checksum, None);
    assert_eq!(legacy.bytes().unwrap(), b"#0,MED\r\n");
    preset.checksum = Some(Checksum {
        skip_first: true,
        output: Output::StarHex,
    });
    let restored: Preset = serde_json::from_str(&serde_json::to_string(&preset).unwrap()).unwrap();
    assert_eq!(restored, preset);
    assert_eq!(restored.bytes().unwrap(), b"#0,MED*50\r\n");
    assert!(serde_json::to_value(&legacy)
        .unwrap()
        .get("checksum")
        .is_none());
    let old: signal_forge::workspace::SavedTerminal = serde_json::from_str("{}").unwrap();
    assert_eq!(old.checksum, None);
    let saved = signal_forge::workspace::SavedTerminal {
        checksum: preset.checksum,
        ..Default::default()
    };
    let restored: signal_forge::workspace::SavedTerminal =
        serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert_eq!(restored.checksum, preset.checksum);
}
