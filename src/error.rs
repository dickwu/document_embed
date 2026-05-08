use crate::id_codec::PdfwmCodecError;

#[derive(Debug, thiserror::Error)]
pub enum PdfwmError {
    #[error("{0}")]
    InvalidArgument(String),
    #[error("{0}")]
    InvalidId(String),
    #[error("{0}")]
    IdTooLong(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Pdf(String),
    #[error("{0}")]
    Image(String),
    #[error("{0}")]
    Watermark(String),
    #[error("{0}")]
    AmbiguousWatermark(String),
    #[error("{0}")]
    Limit(String),
}

impl From<PdfwmCodecError> for PdfwmError {
    fn from(value: PdfwmCodecError) -> Self {
        match value {
            PdfwmCodecError::InvalidId(message) => Self::InvalidId(message),
            PdfwmCodecError::IdTooLong(message) => Self::IdTooLong(message),
            PdfwmCodecError::InvalidPayload(message) => Self::Watermark(message),
        }
    }
}
