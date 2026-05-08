use std::{env, path::PathBuf, str::FromStr};

use ext_php_rs::types::{ZendHashTable, Zval};

use crate::{
    error::PdfwmError,
    id_codec::{IdCodec, IdEncodeOptions, TrustmarkVersion},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdCodecSelection {
    Auto,
    Specific(IdCodec),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputImageFormat {
    Png,
    Jpeg,
}

impl OutputImageFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PdfwmOptions {
    pub id_codec: IdCodecSelection,
    pub model_dir: Option<PathBuf>,
    pub pdfium_lib_path: Option<PathBuf>,
    pub dpi: u16,
    pub strength: f32,
    pub variant: trustmark::Variant,
    pub version: TrustmarkVersion,
    pub image_format: OutputImageFormat,
    pub jpeg_quality: u8,
    pub embed_metadata: bool,
    pub max_pages: usize,
    pub max_pixels_per_page: u64,
    pub max_id_bytes: usize,
    pub allow_control_chars: bool,
    pub min_votes: usize,
}

impl PdfwmOptions {
    pub fn from_php(options: Option<&ZendHashTable>) -> Result<Self, PdfwmError> {
        let id_codec = option_string(options, "id_codec")?
            .or_else(|| env::var("PDFWM_ID_CODEC").ok())
            .unwrap_or_else(|| "auto".to_string());

        let id_codec = if id_codec == "auto" {
            IdCodecSelection::Auto
        } else {
            IdCodecSelection::Specific(IdCodec::from_str(&id_codec).map_err(|err| {
                PdfwmError::InvalidArgument(format!("invalid id_codec option: {err}"))
            })?)
        };

        let model_dir = option_string(options, "model_dir")?
            .or_else(|| env::var("PDFWM_MODEL_DIR").ok())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);

        let pdfium_lib_path = option_string(options, "pdfium_lib_path")?
            .or_else(|| env::var("PDFIUM_DYNAMIC_LIB_PATH").ok())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);

        let version = option_string(options, "version")?
            .or_else(|| env::var("PDFWM_VERSION").ok())
            .unwrap_or_else(|| "BCH_5".to_string());
        let version = TrustmarkVersion::from_str(&version)
            .map_err(|err| PdfwmError::InvalidArgument(format!("invalid version option: {err}")))?;

        let variant = option_string(options, "variant")?
            .or_else(|| env::var("PDFWM_VARIANT").ok())
            .unwrap_or_else(|| "Q".to_string());
        let variant = trustmark::Variant::from_str(&variant).map_err(|_| {
            PdfwmError::InvalidArgument("variant must be one of Q, P, B, or C".to_string())
        })?;

        let image_format =
            option_string(options, "image_format")?.unwrap_or_else(|| "png".to_string());
        let image_format = match image_format.as_str() {
            "png" => OutputImageFormat::Png,
            "jpeg" | "jpg" => OutputImageFormat::Jpeg,
            _ => {
                return Err(PdfwmError::InvalidArgument(
                    "image_format must be png or jpeg".to_string(),
                ));
            }
        };

        Ok(Self {
            id_codec,
            model_dir,
            pdfium_lib_path,
            dpi: option_u16(options, "dpi")?
                .or_else(|| env_u16("PDFWM_DPI"))
                .unwrap_or(180),
            strength: option_f32(options, "strength")?
                .or_else(|| env_f32("PDFWM_STRENGTH"))
                .unwrap_or(0.95),
            variant,
            version,
            image_format,
            jpeg_quality: option_u8(options, "jpeg_quality")?.unwrap_or(92),
            embed_metadata: option_bool(options, "embed_metadata")?.unwrap_or(true),
            max_pages: option_usize(options, "max_pages")?
                .or_else(|| env_usize("PDFWM_MAX_PAGES"))
                .unwrap_or(100),
            max_pixels_per_page: option_u64(options, "max_pixels_per_page")?
                .or_else(|| env_u64("PDFWM_MAX_PIXELS_PER_PAGE"))
                .unwrap_or(50_000_000),
            max_id_bytes: option_usize(options, "max_id_bytes")?
                .or_else(|| env_usize("PDFWM_MAX_ID_BYTES"))
                .unwrap_or(256),
            allow_control_chars: option_bool(options, "allow_control_chars")?.unwrap_or(false),
            min_votes: option_usize(options, "min_votes")?.unwrap_or(1),
        })
    }

    pub fn id_encode_options(&self) -> IdEncodeOptions {
        IdEncodeOptions {
            allow_control_chars: self.allow_control_chars,
            max_id_bytes: self.max_id_bytes,
        }
    }

    pub fn require_model_dir(&self) -> Result<&PathBuf, PdfwmError> {
        let model_dir = self.model_dir.as_ref().ok_or_else(|| {
            PdfwmError::Config("PDFWM_MODEL_DIR or options['model_dir'] is required".to_string())
        })?;

        if !model_dir.is_dir() {
            return Err(PdfwmError::Config(format!(
                "TrustMark model_dir does not exist or is not a directory: {}",
                model_dir.display()
            )));
        }

        Ok(model_dir)
    }
}

fn option_zval<'a>(options: Option<&'a ZendHashTable>, key: &str) -> Option<&'a Zval> {
    options.and_then(|table| table.get(key))
}

fn option_string(options: Option<&ZendHashTable>, key: &str) -> Result<Option<String>, PdfwmError> {
    option_zval(options, key)
        .map(|value| {
            value.str().map(ToString::to_string).ok_or_else(|| {
                PdfwmError::InvalidArgument(format!("options['{key}'] must be a string"))
            })
        })
        .transpose()
}

fn option_bool(options: Option<&ZendHashTable>, key: &str) -> Result<Option<bool>, PdfwmError> {
    option_zval(options, key)
        .map(|value| {
            value.bool().ok_or_else(|| {
                PdfwmError::InvalidArgument(format!("options['{key}'] must be a boolean"))
            })
        })
        .transpose()
}

fn option_u8(options: Option<&ZendHashTable>, key: &str) -> Result<Option<u8>, PdfwmError> {
    option_u64(options, key)?
        .map(|value| {
            u8::try_from(value).map_err(|_| {
                PdfwmError::InvalidArgument(format!("options['{key}'] is out of range for u8"))
            })
        })
        .transpose()
}

fn option_u16(options: Option<&ZendHashTable>, key: &str) -> Result<Option<u16>, PdfwmError> {
    option_u64(options, key)?
        .map(|value| {
            u16::try_from(value).map_err(|_| {
                PdfwmError::InvalidArgument(format!("options['{key}'] is out of range for u16"))
            })
        })
        .transpose()
}

fn option_usize(options: Option<&ZendHashTable>, key: &str) -> Result<Option<usize>, PdfwmError> {
    option_u64(options, key)?
        .map(|value| {
            usize::try_from(value).map_err(|_| {
                PdfwmError::InvalidArgument(format!("options['{key}'] is out of range for usize"))
            })
        })
        .transpose()
}

fn option_u64(options: Option<&ZendHashTable>, key: &str) -> Result<Option<u64>, PdfwmError> {
    option_zval(options, key)
        .map(|value| {
            let long = value.long().ok_or_else(|| {
                PdfwmError::InvalidArgument(format!("options['{key}'] must be an integer"))
            })?;
            u64::try_from(long).map_err(|_| {
                PdfwmError::InvalidArgument(format!("options['{key}'] must be non-negative"))
            })
        })
        .transpose()
}

fn option_f32(options: Option<&ZendHashTable>, key: &str) -> Result<Option<f32>, PdfwmError> {
    option_zval(options, key)
        .map(|value| {
            value
                .double()
                .or_else(|| value.long().map(|value| value as f64))
                .map(|value| value as f32)
                .ok_or_else(|| {
                    PdfwmError::InvalidArgument(format!("options['{key}'] must be a number"))
                })
        })
        .transpose()
}

fn env_usize(key: &str) -> Option<usize> {
    env::var(key).ok()?.parse().ok()
}

fn env_u64(key: &str) -> Option<u64> {
    env::var(key).ok()?.parse().ok()
}

fn env_u16(key: &str) -> Option<u16> {
    env::var(key).ok()?.parse().ok()
}

fn env_f32(key: &str) -> Option<f32> {
    env::var(key).ok()?.parse().ok()
}
