use std::{fmt, str::FromStr};

use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};

const BASE36: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const CODEC_HEADER_BITS: usize = 2;
const LENGTH_BITS: usize = 5;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrustmarkVersion {
    BchSuper,
    #[default]
    Bch5,
    Bch4,
    Bch3,
}

impl TrustmarkVersion {
    pub fn data_bits(self) -> usize {
        match self {
            Self::BchSuper => 40,
            Self::Bch5 => 61,
            Self::Bch4 => 68,
            Self::Bch3 => 75,
        }
    }

    pub fn uint_decimal_max(self) -> BigUint {
        (BigUint::one() << (self.data_bits() - CODEC_HEADER_BITS)) - BigUint::one()
    }

    pub fn decimal_bcd_max_digits(self) -> usize {
        (self.data_bits() - CODEC_HEADER_BITS - LENGTH_BITS) / 4
    }

    pub fn base36_max_chars(self) -> usize {
        max_base36_chars(self.data_bits() - CODEC_HEADER_BITS - LENGTH_BITS)
    }

    pub fn utf8_raw_max_bytes(self) -> usize {
        (self.data_bits() - CODEC_HEADER_BITS) / 8
    }
}

impl fmt::Display for TrustmarkVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::BchSuper => "BCH_SUPER",
            Self::Bch5 => "BCH_5",
            Self::Bch4 => "BCH_4",
            Self::Bch3 => "BCH_3",
        };
        f.write_str(name)
    }
}

impl FromStr for TrustmarkVersion {
    type Err = PdfwmCodecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "BCH_SUPER" => Ok(Self::BchSuper),
            "BCH_5" => Ok(Self::Bch5),
            "BCH_4" => Ok(Self::Bch4),
            "BCH_3" => Ok(Self::Bch3),
            _ => Err(PdfwmCodecError::InvalidId(format!(
                "unsupported TrustMark version: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdCodec {
    Utf8Raw,
    UintDecimal,
    DecimalBcd,
    Base36,
}

impl fmt::Display for IdCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Utf8Raw => "utf8_raw",
            Self::UintDecimal => "uint_decimal",
            Self::DecimalBcd => "decimal_bcd",
            Self::Base36 => "base36",
        };
        f.write_str(name)
    }
}

impl FromStr for IdCodec {
    type Err = PdfwmCodecError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "utf8_raw" => Ok(Self::Utf8Raw),
            "uint_decimal" => Ok(Self::UintDecimal),
            "decimal_bcd" => Ok(Self::DecimalBcd),
            "base36" => Ok(Self::Base36),
            _ => Err(PdfwmCodecError::InvalidId(format!(
                "unsupported id_codec: {value}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdEncodeOptions {
    pub allow_control_chars: bool,
    pub max_id_bytes: usize,
}

impl Default for IdEncodeOptions {
    fn default() -> Self {
        Self {
            allow_control_chars: false,
            max_id_bytes: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedId {
    pub id: String,
    pub codec: IdCodec,
    pub version: TrustmarkVersion,
    pub bits: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedId {
    pub id: String,
    pub codec: IdCodec,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PdfwmCodecError {
    #[error("{0}")]
    InvalidId(String),
    #[error("{0}")]
    IdTooLong(String),
    #[error("{0}")]
    InvalidPayload(String),
}

pub fn encode_id_auto(
    id: &str,
    version: TrustmarkVersion,
    options: IdEncodeOptions,
) -> Result<EncodedId, PdfwmCodecError> {
    validate_common_id(id, options)?;

    for codec in [
        IdCodec::UintDecimal,
        IdCodec::DecimalBcd,
        IdCodec::Base36,
        IdCodec::Utf8Raw,
    ] {
        if let Ok(encoded) = encode_id_with_codec(id, version, codec, options) {
            return Ok(encoded);
        }
    }

    Err(PdfwmCodecError::IdTooLong(capacity_message(version)))
}

pub fn encode_id_with_codec(
    id: &str,
    version: TrustmarkVersion,
    codec: IdCodec,
    options: IdEncodeOptions,
) -> Result<EncodedId, PdfwmCodecError> {
    validate_common_id(id, options)?;

    let bits = match codec {
        IdCodec::Utf8Raw => encode_utf8_raw(id, version)?,
        IdCodec::UintDecimal => encode_uint_decimal(id, version)?,
        IdCodec::DecimalBcd => encode_decimal_bcd(id, version)?,
        IdCodec::Base36 => encode_base36(id, version)?,
    };

    debug_assert_eq!(bits.len(), version.data_bits());

    Ok(EncodedId {
        id: id.to_string(),
        codec,
        version,
        bits,
    })
}

pub fn decode_id(bits: &str) -> Result<DecodedId, PdfwmCodecError> {
    validate_bitstring(bits)?;

    let codec = match &bits[..CODEC_HEADER_BITS] {
        "00" => IdCodec::Utf8Raw,
        "01" => IdCodec::UintDecimal,
        "10" => IdCodec::DecimalBcd,
        "11" => IdCodec::Base36,
        _ => unreachable!("validate_bitstring() only allows binary header bits"),
    };

    let id = match codec {
        IdCodec::Utf8Raw => decode_utf8_raw(bits)?,
        IdCodec::UintDecimal => decode_uint_decimal(bits)?,
        IdCodec::DecimalBcd => decode_decimal_bcd(bits)?,
        IdCodec::Base36 => decode_base36(bits)?,
    };

    Ok(DecodedId { id, codec })
}

pub fn version_from_data_bits(bits: usize) -> Result<TrustmarkVersion, PdfwmCodecError> {
    match bits {
        40 => Ok(TrustmarkVersion::BchSuper),
        61 => Ok(TrustmarkVersion::Bch5),
        68 => Ok(TrustmarkVersion::Bch4),
        75 => Ok(TrustmarkVersion::Bch3),
        _ => Err(PdfwmCodecError::InvalidPayload(format!(
            "unsupported TrustMark data bit length: {bits}"
        ))),
    }
}

pub fn capacity_message(version: TrustmarkVersion) -> String {
    format!(
        "id is too long for TrustMark {version}. Supported direct ID capacity:\n\
         - uint_decimal: 0..{} without leading zeros\n\
         - decimal_bcd: up to {} digits\n\
         - base36: up to {} chars [0-9A-Z]\n\
         - utf8_raw: up to {} UTF-8 bytes\n\
         Use a shorter external ID, switch to BCH_3, or choose numeric/base36 format.",
        version.uint_decimal_max().to_str_radix(10),
        version.decimal_bcd_max_digits(),
        version.base36_max_chars(),
        version.utf8_raw_max_bytes()
    )
}

fn validate_common_id(id: &str, options: IdEncodeOptions) -> Result<(), PdfwmCodecError> {
    if id.is_empty() {
        return Err(PdfwmCodecError::InvalidId(
            "id must not be empty".to_string(),
        ));
    }

    let id_len = id.len();
    if id_len > options.max_id_bytes {
        return Err(PdfwmCodecError::InvalidId(format!(
            "id length {id_len} exceeds max_id_bytes {}",
            options.max_id_bytes
        )));
    }

    if !options.allow_control_chars && id.bytes().any(|byte| byte <= 0x1f || byte == 0x7f) {
        return Err(PdfwmCodecError::InvalidId(
            "id must not contain ASCII control characters".to_string(),
        ));
    }

    Ok(())
}

fn encode_utf8_raw(id: &str, version: TrustmarkVersion) -> Result<String, PdfwmCodecError> {
    if id.as_bytes().contains(&0) {
        return Err(PdfwmCodecError::InvalidId(
            "utf8_raw id must not contain NUL byte".to_string(),
        ));
    }

    let max_bytes = version.utf8_raw_max_bytes();
    if id.len() > max_bytes {
        return Err(PdfwmCodecError::IdTooLong(format!(
            "id is too long for utf8_raw: {} bytes exceeds {max_bytes} bytes for {version}",
            id.len()
        )));
    }

    let mut bits = String::with_capacity(version.data_bits());
    bits.push_str("00");
    for byte in id.bytes() {
        push_u64_bits(&mut bits, u64::from(byte), 8);
    }
    pad_to_data_bits(&mut bits, version);
    Ok(bits)
}

fn decode_utf8_raw(bits: &str) -> Result<String, PdfwmCodecError> {
    let mut bytes = Vec::new();
    for chunk in bits.as_bytes()[CODEC_HEADER_BITS..].chunks(8) {
        if chunk.len() == 8 {
            bytes.push(read_u8_bits(chunk)?);
        }
    }

    while bytes.last() == Some(&0) {
        bytes.pop();
    }

    if bytes.is_empty() {
        return Err(PdfwmCodecError::InvalidPayload(
            "decoded utf8_raw id is empty".to_string(),
        ));
    }

    String::from_utf8(bytes)
        .map_err(|err| PdfwmCodecError::InvalidPayload(format!("invalid UTF-8 payload: {err}")))
}

fn encode_uint_decimal(id: &str, version: TrustmarkVersion) -> Result<String, PdfwmCodecError> {
    if !is_decimal_without_leading_zero(id) {
        return Err(PdfwmCodecError::InvalidId(
            "uint_decimal id must be a non-negative decimal integer without leading zeroes"
                .to_string(),
        ));
    }

    let value = BigUint::parse_bytes(id.as_bytes(), 10).ok_or_else(|| {
        PdfwmCodecError::InvalidId("uint_decimal id is not a valid decimal integer".to_string())
    })?;
    let max = version.uint_decimal_max();
    if value > max {
        return Err(PdfwmCodecError::IdTooLong(format!(
            "uint_decimal id exceeds capacity for {version}; max is {}",
            max.to_str_radix(10)
        )));
    }

    let mut bits = String::with_capacity(version.data_bits());
    bits.push_str("01");
    push_biguint_fixed_width(&mut bits, &value, version.data_bits() - CODEC_HEADER_BITS);
    Ok(bits)
}

fn decode_uint_decimal(bits: &str) -> Result<String, PdfwmCodecError> {
    Ok(bits_to_biguint(&bits.as_bytes()[CODEC_HEADER_BITS..])?.to_str_radix(10))
}

fn encode_decimal_bcd(id: &str, version: TrustmarkVersion) -> Result<String, PdfwmCodecError> {
    if !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(PdfwmCodecError::InvalidId(
            "decimal_bcd id must contain only decimal digits".to_string(),
        ));
    }

    let max_digits = version.decimal_bcd_max_digits();
    if id.len() > max_digits {
        return Err(PdfwmCodecError::IdTooLong(format!(
            "decimal_bcd id has {} digits, exceeding {max_digits} digits for {version}",
            id.len()
        )));
    }

    let mut bits = String::with_capacity(version.data_bits());
    bits.push_str("10");
    push_u64_bits(&mut bits, id.len() as u64, LENGTH_BITS);
    for digit in id.bytes() {
        push_u64_bits(&mut bits, u64::from(digit - b'0'), 4);
    }
    pad_to_data_bits(&mut bits, version);
    Ok(bits)
}

fn decode_decimal_bcd(bits: &str) -> Result<String, PdfwmCodecError> {
    if bits.len() < CODEC_HEADER_BITS + LENGTH_BITS {
        return Err(PdfwmCodecError::InvalidPayload(
            "decimal_bcd payload is too short".to_string(),
        ));
    }

    let len = read_u8_bits(&bits.as_bytes()[CODEC_HEADER_BITS..CODEC_HEADER_BITS + LENGTH_BITS])?
        as usize;
    if len == 0 {
        return Err(PdfwmCodecError::InvalidPayload(
            "decoded decimal_bcd id is empty".to_string(),
        ));
    }

    let mut cursor = CODEC_HEADER_BITS + LENGTH_BITS;
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        if cursor + 4 > bits.len() {
            return Err(PdfwmCodecError::InvalidPayload(
                "decimal_bcd payload ended before all digits were decoded".to_string(),
            ));
        }

        let digit = read_u8_bits(&bits.as_bytes()[cursor..cursor + 4])?;
        if digit > 9 {
            return Err(PdfwmCodecError::InvalidPayload(format!(
                "invalid BCD digit: {digit}"
            )));
        }
        out.push(char::from(b'0' + digit));
        cursor += 4;
    }

    Ok(out)
}

fn encode_base36(id: &str, version: TrustmarkVersion) -> Result<String, PdfwmCodecError> {
    if !id.bytes().all(|byte| BASE36.contains(&byte)) {
        return Err(PdfwmCodecError::InvalidId(
            "base36 id must contain only 0-9A-Z".to_string(),
        ));
    }

    let max_chars = version.base36_max_chars();
    if id.len() > max_chars {
        return Err(PdfwmCodecError::IdTooLong(format!(
            "base36 id has {} chars, exceeding {max_chars} chars for {version}",
            id.len()
        )));
    }

    let value_bits = version.data_bits() - CODEC_HEADER_BITS - LENGTH_BITS;
    let value = parse_base36(id)?;
    let limit = BigUint::one() << value_bits;
    if value >= limit {
        return Err(PdfwmCodecError::IdTooLong(format!(
            "base36 value exceeds {value_bits}-bit capacity for {version}"
        )));
    }

    let mut bits = String::with_capacity(version.data_bits());
    bits.push_str("11");
    push_u64_bits(&mut bits, id.len() as u64, LENGTH_BITS);
    push_biguint_fixed_width(&mut bits, &value, value_bits);
    Ok(bits)
}

fn decode_base36(bits: &str) -> Result<String, PdfwmCodecError> {
    if bits.len() < CODEC_HEADER_BITS + LENGTH_BITS {
        return Err(PdfwmCodecError::InvalidPayload(
            "base36 payload is too short".to_string(),
        ));
    }

    let len = read_u8_bits(&bits.as_bytes()[CODEC_HEADER_BITS..CODEC_HEADER_BITS + LENGTH_BITS])?
        as usize;
    if len == 0 {
        return Err(PdfwmCodecError::InvalidPayload(
            "decoded base36 id is empty".to_string(),
        ));
    }

    let value = bits_to_biguint(&bits.as_bytes()[CODEC_HEADER_BITS + LENGTH_BITS..])?;
    let mut out = value.to_str_radix(36).to_uppercase();
    while out.len() < len {
        out.insert(0, '0');
    }

    if out.len() != len {
        return Err(PdfwmCodecError::InvalidPayload(
            "decoded base36 length mismatch".to_string(),
        ));
    }

    Ok(out)
}

fn is_decimal_without_leading_zero(id: &str) -> bool {
    id == "0"
        || (id
            .bytes()
            .next()
            .is_some_and(|byte| (b'1'..=b'9').contains(&byte))
            && id.bytes().all(|byte| byte.is_ascii_digit()))
}

fn parse_base36(id: &str) -> Result<BigUint, PdfwmCodecError> {
    let mut value = BigUint::zero();
    let radix = BigUint::from(36u8);

    for byte in id.bytes() {
        let digit = BASE36
            .iter()
            .position(|candidate| *candidate == byte)
            .ok_or_else(|| PdfwmCodecError::InvalidId("invalid base36 digit".to_string()))?;
        value *= &radix;
        value += BigUint::from(digit);
    }

    Ok(value)
}

fn max_base36_chars(value_bits: usize) -> usize {
    let limit = BigUint::one() << value_bits;
    let radix = BigUint::from(36u8);
    let mut pow = BigUint::one();
    let mut len = 0;

    while &pow * &radix <= limit {
        pow *= &radix;
        len += 1;
    }

    len
}

fn push_u64_bits(bits: &mut String, value: u64, width: usize) {
    for shift in (0..width).rev() {
        bits.push(if (value >> shift) & 1 == 1 { '1' } else { '0' });
    }
}

fn push_biguint_fixed_width(bits: &mut String, value: &BigUint, width: usize) {
    for shift in (0..width).rev() {
        let bit = (value >> shift) & BigUint::one();
        bits.push(if bit.is_one() { '1' } else { '0' });
    }
}

fn bits_to_biguint(bits: &[u8]) -> Result<BigUint, PdfwmCodecError> {
    let mut value = BigUint::zero();
    for bit in bits {
        value <<= 1usize;
        match bit {
            b'0' => {}
            b'1' => value += BigUint::one(),
            _ => {
                return Err(PdfwmCodecError::InvalidPayload(
                    "payload contains non-binary bits".to_string(),
                ));
            }
        }
    }
    Ok(value)
}

fn read_u8_bits(bits: &[u8]) -> Result<u8, PdfwmCodecError> {
    let value = bits_to_biguint(bits)?;
    value.to_u8().ok_or_else(|| {
        PdfwmCodecError::InvalidPayload("payload field does not fit in u8".to_string())
    })
}

fn pad_to_data_bits(bits: &mut String, version: TrustmarkVersion) {
    while bits.len() < version.data_bits() {
        bits.push('0');
    }
}

fn validate_bitstring(bits: &str) -> Result<(), PdfwmCodecError> {
    if bits.len() < CODEC_HEADER_BITS {
        return Err(PdfwmCodecError::InvalidPayload(
            "payload must contain at least a 2-bit codec header".to_string(),
        ));
    }

    if !bits.bytes().all(|byte| matches!(byte, b'0' | b'1')) {
        return Err(PdfwmCodecError::InvalidPayload(
            "payload must contain only 0 and 1".to_string(),
        ));
    }

    Ok(())
}
