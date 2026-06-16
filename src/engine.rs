use std::{collections::HashMap, fs::File, io::BufWriter, path::Path};

use image::{DynamicImage, ImageFormat, codecs::jpeg::JpegEncoder};
use pdfium_render::prelude::*;
use printpdf::{
    ImageCompression, ImageOptimizationOptions, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Pt,
    RawImage, XObjectTransform,
};
use trustmark::Trustmark;

use crate::{
    error::PdfwmError,
    id_codec::{
        DecodedId, EncodedId, IdCodec, TrustmarkVersion, decode_id, encode_id_auto,
        encode_id_with_codec, version_from_data_bits,
    },
    metadata,
    options::{IdCodecSelection, OutputImageFormat, PdfwmOptions},
};

#[derive(Debug, Clone)]
pub struct EmbedImageResult {
    pub input_path: String,
    pub output_path: String,
    pub id: String,
    pub id_codec: IdCodec,
    pub version: TrustmarkVersion,
    pub variant: trustmark::Variant,
    pub data_bits: usize,
}

#[derive(Debug, Clone)]
pub struct EmbedPdfResult {
    pub input_path: String,
    pub output_path: String,
    pub id: String,
    pub id_codec: IdCodec,
    pub version: TrustmarkVersion,
    pub variant: trustmark::Variant,
    pub data_bits: usize,
    pub page_count: usize,
    pub image_only_pdf: bool,
}

#[derive(Debug, Clone)]
pub struct ExtractImageResult {
    pub id: String,
    pub id_codec: IdCodec,
    pub version: TrustmarkVersion,
    pub variant: trustmark::Variant,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct ExtractPdfResult {
    pub id: String,
    pub id_codec: IdCodec,
    pub version: TrustmarkVersion,
    pub variant: trustmark::Variant,
    pub page_count: usize,
    pub votes: usize,
    pub page_results: Vec<PageExtractResult>,
}

#[derive(Debug, Clone)]
pub struct PageExtractResult {
    pub page: usize,
    pub ok: bool,
    pub id: Option<String>,
    pub id_codec: Option<IdCodec>,
    pub version: Option<TrustmarkVersion>,
    pub confidence: Option<f64>,
    pub error: Option<String>,
}

struct RenderedPage {
    image: DynamicImage,
    width_points: f32,
    height_points: f32,
}

pub fn embed_image_path(
    input_path: &str,
    id: &str,
    output_path: &str,
    options: &PdfwmOptions,
) -> Result<EmbedImageResult, PdfwmError> {
    validate_path(input_path, "inputImagePath")?;
    validate_path(output_path, "outputImagePath")?;

    let encoded = encode_id(id, options)?;
    let tm = trustmark_engine(options)?;
    let input = decode_image_any_format(input_path)?;
    let output = tm
        .encode(encoded.bits.clone(), input, options.strength)
        .map_err(|err| PdfwmError::Watermark(format!("TrustMark encode failed: {err}")))?;
    save_image(&output, output_path, options)?;

    Ok(EmbedImageResult {
        input_path: input_path.to_string(),
        output_path: output_path.to_string(),
        id: id.to_string(),
        id_codec: encoded.codec,
        version: options.version,
        variant: options.variant,
        data_bits: options.version.data_bits(),
    })
}

pub fn extract_image_path(
    image_path: &str,
    options: &PdfwmOptions,
) -> Result<ExtractImageResult, PdfwmError> {
    validate_path(image_path, "imagePath")?;

    let tm = trustmark_engine(options)?;
    let input = decode_image_any_format(image_path)?;
    let bits = tm
        .decode(input)
        .map_err(|err| PdfwmError::Watermark(format!("TrustMark decode failed: {err}")))?;
    let version = version_from_data_bits(bits.len())?;
    let decoded = decode_id(&bits)?;

    Ok(ExtractImageResult {
        id: decoded.id,
        id_codec: decoded.codec,
        version,
        variant: options.variant,
        confidence: 1.0,
    })
}

pub fn embed_pdf_path(
    input_path: &str,
    id: &str,
    output_path: &str,
    options: &PdfwmOptions,
) -> Result<EmbedPdfResult, PdfwmError> {
    validate_path(input_path, "inputPdfPath")?;
    validate_path(output_path, "outputPdfPath")?;

    let encoded = encode_id(id, options)?;
    let tm = trustmark_engine(options)?;
    let pages = render_pdf_pages(input_path, options)?;
    let page_count = pages.len();
    let mut watermarked_pages = Vec::with_capacity(page_count);

    for page in pages {
        let watermarked = tm
            .encode(encoded.bits.clone(), page.image, options.strength)
            .map_err(|err| PdfwmError::Watermark(format!("TrustMark encode failed: {err}")))?;
        // Collapse to 8-bit RGB immediately. TrustMark hands back a 32-bit-float
        // image (~5x the size of RGB8); holding that for every page until the
        // PDF rebuild is what drove peak memory to several GB on multi-page,
        // high-DPI exports. The rebuild down-converts to RGB8 anyway, so doing
        // it here is output-identical but frees the float buffer per page.
        let watermarked = DynamicImage::ImageRgb8(watermarked.to_rgb8());
        watermarked_pages.push((watermarked, page.width_points, page.height_points));
    }

    rebuild_image_only_pdf(&watermarked_pages, output_path, options)?;
    if options.embed_metadata {
        metadata::write_pdf_metadata(
            output_path,
            id,
            encoded.codec,
            options.version,
            options.variant,
        )?;
    }

    Ok(EmbedPdfResult {
        input_path: input_path.to_string(),
        output_path: output_path.to_string(),
        id: id.to_string(),
        id_codec: encoded.codec,
        version: options.version,
        variant: options.variant,
        data_bits: options.version.data_bits(),
        page_count,
        image_only_pdf: true,
    })
}

pub fn extract_pdf_path(
    pdf_path: &str,
    options: &PdfwmOptions,
) -> Result<ExtractPdfResult, PdfwmError> {
    validate_path(pdf_path, "pdfPath")?;

    let tm = trustmark_engine(options)?;
    let pages = render_pdf_pages(pdf_path, options)?;
    let page_count = pages.len();
    let mut page_results = Vec::with_capacity(page_count);

    for (index, page) in pages.into_iter().enumerate() {
        let page_number = index + 1;
        let result = match tm.decode(page.image) {
            Ok(bits) => match decode_id(&bits) {
                Ok(decoded) => PageExtractResult {
                    page: page_number,
                    ok: true,
                    id: Some(decoded.id),
                    id_codec: Some(decoded.codec),
                    version: version_from_data_bits(bits.len()).ok(),
                    confidence: Some(1.0),
                    error: None,
                },
                Err(err) => PageExtractResult {
                    page: page_number,
                    ok: false,
                    id: None,
                    id_codec: None,
                    version: None,
                    confidence: None,
                    error: Some(err.to_string()),
                },
            },
            Err(err) => PageExtractResult {
                page: page_number,
                ok: false,
                id: None,
                id_codec: None,
                version: None,
                confidence: None,
                error: Some(format!("TrustMark decode failed: {err}")),
            },
        };
        page_results.push(result);
    }

    let (decoded, version, votes) = choose_pdf_vote(&page_results, options.min_votes)?;

    Ok(ExtractPdfResult {
        id: decoded.id,
        id_codec: decoded.codec,
        version,
        variant: options.variant,
        page_count,
        votes,
        page_results,
    })
}

fn encode_id(id: &str, options: &PdfwmOptions) -> Result<EncodedId, PdfwmError> {
    match options.id_codec {
        IdCodecSelection::Auto => encode_id_auto(id, options.version, options.id_encode_options()),
        IdCodecSelection::Specific(codec) => {
            encode_id_with_codec(id, options.version, codec, options.id_encode_options())
        }
    }
    .map_err(Into::into)
}

fn trustmark_engine(options: &PdfwmOptions) -> Result<Trustmark, PdfwmError> {
    let model_dir = options.require_model_dir()?;
    Trustmark::new(
        model_dir,
        options.variant,
        to_trustmark_version(options.version),
    )
    .map_err(|err| PdfwmError::Config(format!("failed to load TrustMark models: {err}")))
}

fn to_trustmark_version(version: TrustmarkVersion) -> trustmark::Version {
    match version {
        TrustmarkVersion::BchSuper => trustmark::Version::BchSuper,
        TrustmarkVersion::Bch5 => trustmark::Version::Bch5,
        TrustmarkVersion::Bch4 => trustmark::Version::Bch4,
        TrustmarkVersion::Bch3 => trustmark::Version::Bch3,
    }
}

fn save_image(
    image: &DynamicImage,
    output_path: &str,
    options: &PdfwmOptions,
) -> Result<(), PdfwmError> {
    match options.image_format {
        OutputImageFormat::Png => DynamicImage::ImageRgba8(image.to_rgba8())
            .save_with_format(output_path, ImageFormat::Png)
            .map_err(|err| PdfwmError::Image(format!("failed to write PNG: {err}"))),
        OutputImageFormat::Jpeg => {
            let file = File::create(output_path)
                .map_err(|err| PdfwmError::Image(format!("failed to create JPEG: {err}")))?;
            let mut writer = BufWriter::new(file);
            let rgb = image.to_rgb8();
            let mut encoder = JpegEncoder::new_with_quality(&mut writer, options.jpeg_quality);
            encoder
                .encode_image(&DynamicImage::ImageRgb8(rgb))
                .map_err(|err| PdfwmError::Image(format!("failed to write JPEG: {err}")))
        }
    }
}

/// Decode an image, detecting its format from the file *content* (magic bytes)
/// rather than the filename extension.
///
/// Callers such as the admin "Leak Check" upload path hand us temporary files
/// whose name carries a random, opaque suffix (e.g. `/tmp/phpXXXXXX.EAu0iQ`).
/// The bare `image::open` guesses the format from that suffix and aborts with
/// `The file extension ."EAu0iQ" was not recognized as an image format`, even
/// for a perfectly valid PNG/JPEG/WebP. `with_guessed_format` instead sniffs the
/// leading bytes, so any supported raster format decodes regardless of how the
/// file happens to be named on disk.
fn decode_image_any_format(path: &str) -> Result<DynamicImage, PdfwmError> {
    image::ImageReader::open(path)
        .map_err(|err| PdfwmError::Image(format!("failed to open input image: {err}")))?
        .with_guessed_format()
        .map_err(|err| PdfwmError::Image(format!("failed to read input image: {err}")))?
        .decode()
        .map_err(|err| PdfwmError::Image(format!("failed to decode input image: {err}")))
}

fn render_pdf_pages(
    pdf_path: &str,
    options: &PdfwmOptions,
) -> Result<Vec<RenderedPage>, PdfwmError> {
    let pdfium = bind_pdfium(options)?;
    let document = pdfium
        .load_pdf_from_file(pdf_path, None)
        .map_err(|err| PdfwmError::Pdf(format!("failed to open PDF: {err}")))?;
    let page_count = document.pages().len() as usize;

    if page_count == 0 {
        return Err(PdfwmError::Pdf("PDF has no pages".to_string()));
    }

    if page_count > options.max_pages {
        return Err(PdfwmError::Limit(format!(
            "PDF has {page_count} pages, exceeding max_pages {}",
            options.max_pages
        )));
    }

    let mut pages = Vec::with_capacity(page_count);
    for page in document.pages().iter() {
        let width_points = page.width().value;
        let height_points = page.height().value;
        let target_width = points_to_pixels(width_points, options.dpi);
        let target_height = points_to_pixels(height_points, options.dpi);
        let pixels = u64::from(target_width as u32) * u64::from(target_height as u32);

        if pixels > options.max_pixels_per_page {
            return Err(PdfwmError::Limit(format!(
                "rendered page would be {pixels} pixels, exceeding max_pixels_per_page {}",
                options.max_pixels_per_page
            )));
        }

        let render_config = PdfRenderConfig::new().set_target_size(target_width, target_height);
        let image = page
            .render_with_config(&render_config)
            .map_err(|err| PdfwmError::Pdf(format!("failed to render PDF page: {err}")))?
            .as_image()
            .map_err(|err| {
                PdfwmError::Pdf(format!("failed to convert PDF page to image: {err}"))
            })?;

        pages.push(RenderedPage {
            image,
            width_points,
            height_points,
        });
    }

    Ok(pages)
}

fn bind_pdfium(options: &PdfwmOptions) -> Result<Pdfium, PdfwmError> {
    let bindings = if let Some(path) = options.pdfium_lib_path.as_ref() {
        let library_path = if path.is_dir() {
            Pdfium::pdfium_platform_library_name_at_path(path)
        } else {
            path.clone()
        };
        match Pdfium::bind_to_library(&library_path) {
            Ok(bindings) => bindings,
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => {
                return Ok(Pdfium::default());
            }
            Err(err) => {
                return Err(PdfwmError::Config(format!(
                    "failed to bind PDFium at {}: {err}",
                    library_path.display()
                )));
            }
        }
    } else {
        match Pdfium::bind_to_system_library() {
            Ok(bindings) => bindings,
            Err(PdfiumError::PdfiumLibraryBindingsAlreadyInitialized) => {
                return Ok(Pdfium::default());
            }
            Err(err) => {
                return Err(PdfwmError::Config(format!(
                    "PDFIUM_DYNAMIC_LIB_PATH or options['pdfium_lib_path'] is required unless PDFium is installed system-wide: {err}"
                )));
            }
        }
    };

    Ok(Pdfium::new(bindings))
}

fn rebuild_image_only_pdf(
    pages: &[(DynamicImage, f32, f32)],
    output_path: &str,
    options: &PdfwmOptions,
) -> Result<(), PdfwmError> {
    let mut doc = PdfDocument::new("pdfwm image-only output");
    doc.metadata.info.creator = "pdfwm".to_string();
    doc.metadata.info.producer = "pdfwm".to_string();
    let mut pdf_pages = Vec::with_capacity(pages.len());

    for (image, width_points, height_points) in pages {
        let raw_image = RawImage::from_dynamic_image(DynamicImage::ImageRgb8(image.to_rgb8()))
            .map_err(PdfwmError::Pdf)?;
        let image_id = doc.add_image(&raw_image);
        let ops = vec![Op::UseXobject {
            id: image_id,
            transform: XObjectTransform {
                translate_x: Some(Pt(0.0)),
                translate_y: Some(Pt(0.0)),
                rotate: None,
                scale_x: None,
                scale_y: None,
                dpi: Some(f32::from(options.dpi)),
            },
        }];
        pdf_pages.push(PdfPage::new(
            Mm(points_to_mm(*width_points)),
            Mm(points_to_mm(*height_points)),
            ops,
        ));
    }

    // CRITICAL for output sharpness. `PdfSaveOptions::default()` enables image
    // optimization with `max_image_size = "2MB"`, where the budget is measured
    // against the *uncompressed* pixel buffer. printpdf therefore downsamples
    // every page to ~2 MB / 3 bytes ≈ 0.7 MP (~735x951, i.e. ~86 effective DPI)
    // no matter how high we rasterize — that silent cap, not the source DPI, was
    // the real cause of the blurry exports. Drop the cap so each page keeps its
    // full rasterized resolution, and compress with a high-quality JPEG so the
    // file stays a sane size. The TrustMark watermark is JPEG/screenshot-robust,
    // so lossy compression at this quality does not break extraction.
    let jpeg_quality = (f32::from(options.jpeg_quality) / 100.0).clamp(0.0, 1.0);
    let save_options = PdfSaveOptions {
        image_optimization: Some(ImageOptimizationOptions {
            quality: Some(jpeg_quality),
            // None = never downscale. This is the line that fixes the blur.
            max_image_size: None,
            dither_greyscale: Some(false),
            convert_to_greyscale: Some(false),
            auto_optimize: Some(false),
            format: Some(ImageCompression::Jpeg),
        }),
        ..PdfSaveOptions::default()
    };

    let mut warnings = Vec::new();
    let bytes = doc.with_pages(pdf_pages).save(&save_options, &mut warnings);
    std::fs::write(output_path, bytes)
        .map_err(|err| PdfwmError::Pdf(format!("failed to write output PDF: {err}")))?;

    Ok(())
}

fn choose_pdf_vote(
    page_results: &[PageExtractResult],
    min_votes: usize,
) -> Result<(DecodedId, TrustmarkVersion, usize), PdfwmError> {
    let mut votes: HashMap<(String, IdCodec, TrustmarkVersion), (usize, f64)> = HashMap::new();

    for page in page_results.iter().filter(|page| page.ok) {
        if let (Some(id), Some(codec), Some(version), Some(confidence)) = (
            page.id.as_ref(),
            page.id_codec,
            page.version,
            page.confidence,
        ) {
            let entry = votes
                .entry((id.clone(), codec, version))
                .or_insert((0, 0.0));
            entry.0 += 1;
            entry.1 += confidence;
        }
    }

    if votes.is_empty() {
        return Err(PdfwmError::Watermark(
            "no TrustMark watermark could be decoded from any PDF page".to_string(),
        ));
    }

    let mut ranked = votes.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        let vote_cmp = b.1.0.cmp(&a.1.0);
        if vote_cmp.is_eq() {
            let a_avg = a.1.1 / a.1.0 as f64;
            let b_avg = b.1.1 / b.1.0 as f64;
            b_avg
                .partial_cmp(&a_avg)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else {
            vote_cmp
        }
    });

    let ((id, codec, version), (vote_count, confidence_sum)) = ranked[0].clone();
    if vote_count < min_votes {
        return Err(PdfwmError::Watermark(format!(
            "decoded watermark received {vote_count} votes, below min_votes {min_votes}"
        )));
    }

    if ranked.len() > 1 {
        let top_avg = confidence_sum / vote_count as f64;
        let (_, (second_votes, second_confidence_sum)) = &ranked[1];
        let second_avg = *second_confidence_sum / *second_votes as f64;
        if vote_count == *second_votes && (top_avg - second_avg).abs() < f64::EPSILON {
            return Err(PdfwmError::AmbiguousWatermark(
                "PDF pages contain conflicting watermarks with equal vote strength".to_string(),
            ));
        }
    }

    Ok((DecodedId { id, codec }, version, vote_count))
}

fn validate_path(path: &str, name: &str) -> Result<(), PdfwmError> {
    if path.trim().is_empty() {
        return Err(PdfwmError::InvalidArgument(format!(
            "{name} must not be empty"
        )));
    }
    Ok(())
}

fn points_to_pixels(points: f32, dpi: u16) -> i32 {
    ((points / 72.0) * f32::from(dpi)).ceil().max(1.0) as i32
}

fn points_to_mm(points: f32) -> f32 {
    points / 72.0 * 25.4
}

#[allow(dead_code)]
fn ensure_parent_exists(path: &Path) -> Result<(), PdfwmError> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            return Err(PdfwmError::InvalidArgument(format!(
                "output parent directory does not exist: {}",
                parent.display()
            )));
        }
    }
    Ok(())
}
