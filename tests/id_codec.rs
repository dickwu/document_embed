use pdfwm::id_codec::{
    IdCodec, IdEncodeOptions, TrustmarkVersion, decode_id, encode_id_auto, encode_id_with_codec,
    version_from_data_bits,
};

fn assert_roundtrip(id: &str, version: TrustmarkVersion, codec: IdCodec) {
    let encoded = encode_id_with_codec(id, version, codec, IdEncodeOptions::default()).unwrap();
    assert_eq!(encoded.bits.len(), version.data_bits());
    assert_eq!(encoded.codec, codec);

    let decoded = decode_id(&encoded.bits).unwrap();
    assert_eq!(decoded.id, id);
    assert_eq!(decoded.codec, codec);
}

fn assert_auto_roundtrip(id: &str, version: TrustmarkVersion, codec: IdCodec) {
    let encoded = encode_id_auto(id, version, IdEncodeOptions::default()).unwrap();
    assert_eq!(encoded.bits.len(), version.data_bits());
    assert_eq!(encoded.codec, codec);

    let decoded = decode_id(&encoded.bits).unwrap();
    assert_eq!(decoded.id, id);
    assert_eq!(decoded.codec, codec);
}

#[test]
fn uint_decimal_roundtrips_and_enforces_bch5_capacity() {
    assert_auto_roundtrip("0", TrustmarkVersion::Bch5, IdCodec::UintDecimal);
    assert_auto_roundtrip("123456789", TrustmarkVersion::Bch5, IdCodec::UintDecimal);
    assert_auto_roundtrip(
        "576460752303423487",
        TrustmarkVersion::Bch5,
        IdCodec::UintDecimal,
    );

    assert!(
        encode_id_with_codec(
            "576460752303423488",
            TrustmarkVersion::Bch5,
            IdCodec::UintDecimal,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
}

#[test]
fn decimal_bcd_preserves_leading_zeroes_and_enforces_bch5_capacity() {
    assert_auto_roundtrip("000123", TrustmarkVersion::Bch5, IdCodec::DecimalBcd);
    assert_auto_roundtrip("001234567890", TrustmarkVersion::Bch5, IdCodec::DecimalBcd);

    assert!(
        encode_id_with_codec(
            "00012345678901",
            TrustmarkVersion::Bch5,
            IdCodec::DecimalBcd,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
}

#[test]
fn base36_roundtrips_and_rejects_lowercase() {
    assert_auto_roundtrip("A1B2C3D4", TrustmarkVersion::Bch5, IdCodec::Base36);
    assert_auto_roundtrip("USER123456", TrustmarkVersion::Bch5, IdCodec::Base36);

    assert!(
        encode_id_with_codec(
            "USER1234567",
            TrustmarkVersion::Bch5,
            IdCodec::Base36,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
    assert!(
        encode_id_with_codec(
            "abc",
            TrustmarkVersion::Bch5,
            IdCodec::Base36,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
}

#[test]
fn utf8_raw_roundtrips_and_counts_bytes() {
    assert_roundtrip("ABC123", TrustmarkVersion::Bch5, IdCodec::Utf8Raw);
    assert_roundtrip("用户123", TrustmarkVersion::Bch3, IdCodec::Utf8Raw);

    assert!(
        encode_id_with_codec(
            "用户123",
            TrustmarkVersion::Bch5,
            IdCodec::Utf8Raw,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
    assert!(
        encode_id_with_codec(
            "user_123456",
            TrustmarkVersion::Bch5,
            IdCodec::Utf8Raw,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
    assert!(
        encode_id_with_codec(
            "ABC\0",
            TrustmarkVersion::Bch5,
            IdCodec::Utf8Raw,
            IdEncodeOptions::default(),
        )
        .is_err()
    );
}

#[test]
fn common_validation_rejects_empty_and_control_chars() {
    assert!(encode_id_auto("", TrustmarkVersion::Bch5, IdEncodeOptions::default()).is_err());
    assert!(
        encode_id_auto(
            "ABC\n123",
            TrustmarkVersion::Bch5,
            IdEncodeOptions::default()
        )
        .is_err()
    );
}

#[test]
fn version_can_be_inferred_from_data_bit_length() {
    assert_eq!(
        version_from_data_bits(40).unwrap(),
        TrustmarkVersion::BchSuper
    );
    assert_eq!(version_from_data_bits(61).unwrap(), TrustmarkVersion::Bch5);
    assert_eq!(version_from_data_bits(68).unwrap(), TrustmarkVersion::Bch4);
    assert_eq!(version_from_data_bits(75).unwrap(), TrustmarkVersion::Bch3);
    assert!(version_from_data_bits(62).is_err());
}
