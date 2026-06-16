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
    /// Target side length (px) of each watermark tile. The page is split into a
    /// grid of ~square tiles and each is watermarked with the same id, so any
    /// partial screenshot containing one whole tile is still traceable and the
    /// TrustMark residual is upscaled only ~tile/256x (not ~page/256x), which
    /// makes the faint background "shadow" far finer. 0 disables tiling.
    pub tile_size: u32,
    /// Width (px) of the residual edge-feather inside each tile. 0 = derive from
    /// the tile size. Feathering the residual to zero at tile borders keeps the
    /// tiles seamless (no visible grid) and avoids hard watermark discontinuities.
    pub tile_feather: u32,
    /// Whether extract should run the sliding-window search (needed to decode
    /// tiled documents and partial screenshots). Disable for legacy whole-image
    /// decode only.
    pub search_decode: bool,
    /// Minimum number of agreeing window decodes required to trust a watermark
    /// id from the search (defends against rare BCH false positives). A single
    /// whole-image decode is still honoured as a backward-compatible fallback.
    pub min_search_votes: usize,
    /// Upper bound on how many windows the search scans per image (caps the
    /// worst-case decode time on very large uploads).
    pub max_search_windows: usize,
    /// When set, reject a decoded numeric id with more than this many digits.
    /// TrustMark's decoder always emits 100 bits and BCH "corrects" even an
    /// unwatermarked region into a *consistent* bogus id; constraining the
    /// result to the id space we actually embed (small auto-increment audit
    /// ids) is what stops a clean page from being read as watermarked. Paired
    /// with an `id_codec` of `uint_decimal`, a 20-digit or base36 "id" is
    /// recognised as noise and discarded.
    pub id_max_digits: Option<usize>,
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
            // High-definition default. Each page is rasterized at this DPI and
            // the TrustMark residual is added on top of the full-resolution
            // raster, so the page is only ever as sharp as this value. 180 DPI
            // looked soft/blurry on screen; 500 DPI keeps text crisp. A 500 DPI
            // US-Letter page is ~23 MP — comfortably under max_pixels_per_page.
            // Override with options['dpi'] or PDFWM_DPI for larger/faster runs.
            dpi: option_u16(options, "dpi")?
                .or_else(|| env_u16("PDFWM_DPI"))
                .unwrap_or(500),
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
            // Raised alongside the 500 DPI default so legitimate large pages
            // (e.g. A3 at 500 DPI ~= 48 MP, or Letter pushed past 500 DPI) are
            // not rejected with a Limit error. Still a guard against pathological
            // page dimensions blowing up memory during rasterization.
            max_pixels_per_page: option_u64(options, "max_pixels_per_page")?
                .or_else(|| env_u64("PDFWM_MAX_PIXELS_PER_PAGE"))
                .unwrap_or(100_000_000),
            max_id_bytes: option_usize(options, "max_id_bytes")?
                .or_else(|| env_usize("PDFWM_MAX_ID_BYTES"))
                .unwrap_or(256),
            allow_control_chars: option_bool(options, "allow_control_chars")?.unwrap_or(false),
            min_votes: option_usize(options, "min_votes")?.unwrap_or(1),
            tile_size: option_u32(options, "tile_size")?
                .or_else(|| env_u32("PDFWM_TILE_SIZE"))
                .unwrap_or(1280),
            tile_feather: option_u32(options, "tile_feather")?
                .or_else(|| env_u32("PDFWM_TILE_FEATHER"))
                .unwrap_or(0),
            search_decode: option_bool(options, "search_decode")?
                .or_else(|| env_bool("PDFWM_SEARCH_DECODE"))
                .unwrap_or(true),
            min_search_votes: option_usize(options, "min_search_votes")?
                .or_else(|| env_usize("PDFWM_MIN_SEARCH_VOTES"))
                .unwrap_or(2),
            max_search_windows: option_usize(options, "max_search_windows")?
                .or_else(|| env_usize("PDFWM_MAX_SEARCH_WINDOWS"))
                .unwrap_or(180),
            id_max_digits: option_usize(options, "id_max_digits")?
                .or_else(|| env_usize("PDFWM_ID_MAX_DIGITS")),
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

fn option_u32(options: Option<&ZendHashTable>, key: &str) -> Result<Option<u32>, PdfwmError> {
    option_u64(options, key)?
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                PdfwmError::InvalidArgument(format!("options['{key}'] is out of range for u32"))
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

fn env_u32(key: &str) -> Option<u32> {
    env::var(key).ok()?.parse().ok()
}

fn env_bool(key: &str) -> Option<bool> {
    match env::var(key).ok()?.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_f32(key: &str) -> Option<f32> {
    env::var(key).ok()?.parse().ok()
}
