use std::{collections::HashMap, fs::File, io::BufWriter, path::Path};

use image::{
    DynamicImage, GenericImageView, ImageFormat, RgbImage, codecs::jpeg::JpegEncoder, imageops,
};
use pdfium_render::prelude::*;
use printpdf::{
    ImageCompression, ImageOptimizationOptions, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Pt,
    RawImage, XObjectTransform,
};
use rayon::prelude::*;
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
    pub votes: usize,
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
    let output = embed_tiled(&tm, &encoded.bits, input, options)?;
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

    // A leaked screenshot is frequently only a *crop* of a page. A tiled
    // document repeats the same id across the page, so the sliding-window
    // search finds and corroborates it from whatever region was captured; the
    // whole-image attempt inside decode_search keeps legacy single-watermark
    // images and full-page shots working unchanged.
    let hits = decode_search(&tm, &input, options);
    let (decoded, version, votes, confidence) = decide_from_hits(&hits, options.min_search_votes)?;

    Ok(ExtractImageResult {
        id: decoded.id,
        id_codec: decoded.codec,
        version,
        variant: options.variant,
        confidence,
        votes,
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

    let dbg = std::env::var("PDFWM_DEBUG_TIME").is_ok();
    let encoded = encode_id(id, options)?;
    let mark = std::time::Instant::now();
    let tm = trustmark_engine(options)?;
    if dbg {
        eprintln!(
            "[t] engine build: {:.0}ms",
            mark.elapsed().as_secs_f64() * 1e3
        );
    }
    let mark = std::time::Instant::now();
    let pages = render_pdf_pages(input_path, options)?;
    let page_count = pages.len();
    if dbg {
        eprintln!(
            "[t] render {page_count} pages: {:.0}ms",
            mark.elapsed().as_secs_f64() * 1e3
        );
    }
    let mut watermarked_pages = Vec::with_capacity(page_count);

    for page in pages {
        // Tiled embed returns an already-RGB8 page. Each tile's TrustMark output
        // (a 32-bit-float image ~5x the size of RGB8) is consumed and collapsed
        // tile-by-tile inside embed_tiled, so peak memory stays bounded on
        // multi-page, high-DPI exports instead of holding floats for every page.
        let mark = std::time::Instant::now();
        let watermarked = embed_tiled(&tm, &encoded.bits, page.image, options)?;
        if dbg {
            eprintln!(
                "[t] embed page: {:.0}ms",
                mark.elapsed().as_secs_f64() * 1e3
            );
        }
        watermarked_pages.push((watermarked, page.width_points, page.height_points));
    }

    let mark = std::time::Instant::now();
    rebuild_image_only_pdf(&watermarked_pages, output_path, options)?;
    if dbg {
        eprintln!("[t] save pdf: {:.0}ms", mark.elapsed().as_secs_f64() * 1e3);
    }
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
    let mut all_hits: Vec<DecodeHit> = Vec::new();

    for (index, page) in pages.into_iter().enumerate() {
        let page_number = index + 1;
        // Search each rendered page the same way as an uploaded screenshot: a
        // tiled page no longer decodes from a single whole-page pass, so the
        // window search is what recovers it. Votes are pooled across all pages.
        let hits = decode_search(&tm, &page.image, options);
        let result = match decide_from_hits(&hits, options.min_search_votes) {
            Ok((decoded, version, _votes, confidence)) => PageExtractResult {
                page: page_number,
                ok: true,
                id: Some(decoded.id),
                id_codec: Some(decoded.codec),
                version: Some(version),
                confidence: Some(confidence),
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
        };
        page_results.push(result);
        all_hits.extend(hits);
    }

    let (decoded, version, votes, _confidence) =
        decide_from_hits(&all_hits, options.min_search_votes)?;

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

/// Largest number of tiles allowed along one page axis (a guard so a
/// pathologically large page cannot explode the embed into thousands of NN
/// passes). With the default 1280px target this only bites past ~20k px sides.
const MAX_TILES_PER_AXIS: u32 = 16;

/// Sliding-window decode: each window side is this fraction of the image's
/// short edge. A whole tile must land inside one window to decode, so the set is
/// built around the fractions a tile naturally occupies in a *full-page* upload
/// — 1/3, 1/2, 1/4, 1/5 of the page — which, paired with a half-window stride,
/// place window edges right on the tile grid. The larger 0.66/0.90 entries cover
/// the other regime, where the upload is itself only a tile or two. The order is
/// most-likely-first so the early-exit in `decode_search` stops fast on the
/// common full-page case.
const SEARCH_FRACTIONS: &[f32] = &[0.33, 0.50, 0.66, 0.90, 0.25, 0.45, 0.20];

/// Fractional overlap between adjacent search windows. 0.5 means a genuine tile
/// is straddled by several windows, which is what produces the corroborating
/// votes that distinguish a real id from a stray BCH false-positive.
const SEARCH_OVERLAP: f32 = 0.5;

/// Windows smaller than this carry too little signal to decode reliably; skip
/// scales that would fall below it.
const MIN_WINDOW: u32 = 192;

/// TrustMark resizes every input to 256x256 before decoding, so there is no
/// benefit to feeding it a multi-megapixel crop — only cost. Each window (and
/// the whole image) is first shrunk so its longer side is at most this, which
/// keeps a single decode at a few tens of ms instead of ~150ms while leaving
/// the watermark (a low-frequency signal) intact.
const DECODE_WORK_SIDE: u32 = 448;

/// Cap on window start positions per axis per scale. Without it a tall crop's
/// finest scale alone would exhaust the whole-image window budget before the
/// search ever reached the scale that aligns to its tiles. Bounding positions
/// per axis spreads the budget so every scale is sampled (breadth over depth).
const MAX_POS_PER_AXIS: u32 = 7;

/// Watermark a page by tiling: split it into a grid of ~square tiles and embed
/// the same id into each one. Spatial redundancy makes partial screenshots
/// traceable, and because each tile's TrustMark residual is upscaled only
/// ~tile/256x (instead of ~page/256x for a whole-page watermark) the residual
/// is far finer — the faint background "shadow" all but disappears. The
/// residual is feathered to zero at every tile border so the grid is seamless.
fn embed_tiled(
    tm: &Trustmark,
    bits: &str,
    page: DynamicImage,
    options: &PdfwmOptions,
) -> Result<DynamicImage, PdfwmError> {
    let base: RgbImage = page.to_rgb8();
    let (width, height) = (base.width(), base.height());

    let cols = tile_count(width, options.tile_size);
    let rows = tile_count(height, options.tile_size);

    // A single tile is byte-for-byte the legacy whole-image watermark.
    if cols <= 1 && rows <= 1 {
        let encoded = tm
            .encode(
                bits.to_string(),
                DynamicImage::ImageRgb8(base),
                options.strength,
            )
            .map_err(|err| PdfwmError::Watermark(format!("TrustMark encode failed: {err}")))?;
        return Ok(DynamicImage::ImageRgb8(encoded.to_rgb8()));
    }

    let xs = tile_bounds(width, cols);
    let ys = tile_bounds(height, rows);

    // Tile rectangles, watermarked in parallel: each tile is an independent
    // TrustMark encode (~80% of an embed's cost) and the page splits cleanly
    // across cores. Compositing the finished tiles back is cheap and sequential.
    let mut jobs = Vec::with_capacity((cols * rows) as usize);
    for ry in 0..rows as usize {
        let (y0, y1) = (ys[ry], ys[ry + 1]);
        for cx in 0..cols as usize {
            let (x0, x1) = (xs[cx], xs[cx + 1]);
            if x1 > x0 && y1 > y0 {
                jobs.push((x0, y0, x1 - x0, y1 - y0));
            }
        }
    }

    let patches = jobs
        .par_iter()
        .map(|&(x0, y0, tile_w, tile_h)| {
            let tile: RgbImage = imageops::crop_imm(&base, x0, y0, tile_w, tile_h).to_image();
            let encoded = tm
                .encode(
                    bits.to_string(),
                    DynamicImage::ImageRgb8(tile.clone()),
                    options.strength,
                )
                .map_err(|err| PdfwmError::Watermark(format!("TrustMark encode failed: {err}")))?
                .to_rgb8();

            let margin = if options.tile_feather > 0 {
                options.tile_feather
            } else {
                (tile_w.min(tile_h) / 16).max(8)
            };

            // Feather the residual to zero at the tile borders and bake it onto a
            // copy of the original tile, so each tile's edges meet the untouched
            // page seamlessly.
            let mut patch = tile.clone();
            for ty in 0..tile_h {
                let weight_y = feather_weight(ty, tile_h, margin);
                for tx in 0..tile_w {
                    let weight = weight_y * feather_weight(tx, tile_w, margin);
                    let src = tile.get_pixel(tx, ty).0;
                    let enc = encoded.get_pixel(tx, ty).0;
                    let px = patch.get_pixel_mut(tx, ty);
                    for ch in 0..3 {
                        let delta = (f32::from(enc[ch]) - f32::from(src[ch])) * weight;
                        px.0[ch] = (f32::from(src[ch]) + delta).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
            Ok((x0, y0, patch))
        })
        .collect::<Result<Vec<_>, PdfwmError>>()?;

    // `base` is no longer borrowed by the parallel encode; reuse it as the canvas.
    let mut out = base;
    for (x0, y0, patch) in patches {
        imageops::replace(&mut out, &patch, i64::from(x0), i64::from(y0));
    }

    Ok(DynamicImage::ImageRgb8(out))
}

/// Number of tiles along an axis of length `total` for a target tile side.
fn tile_count(total: u32, target: u32) -> u32 {
    if target == 0 || total == 0 {
        return 1;
    }
    let n = (f64::from(total) / f64::from(target)).round() as i64;
    (n.max(1) as u32).min(MAX_TILES_PER_AXIS)
}

/// The `n + 1` integer boundary positions splitting `[0, total)` into `n`
/// near-equal segments.
fn tile_bounds(total: u32, n: u32) -> Vec<u32> {
    (0..=n)
        .map(|i| ((u64::from(total) * u64::from(i)) / u64::from(n)) as u32)
        .collect()
}

/// Edge-feather weight in `[0, 1]`: 0 exactly at a tile border, ramping linearly
/// to 1 over `margin` px, flat 1 in the interior. Because both sides of a shared
/// border fall to 0, adjacent tiles meet continuously and leave no visible seam.
#[inline]
fn feather_weight(i: u32, len: u32, margin: u32) -> f32 {
    if margin == 0 || len <= 1 {
        return 1.0;
    }
    let dist = i.min(len - 1 - i) as f32;
    (dist / margin as f32).min(1.0)
}

/// One successful decode, tagged by whether it came from the whole image or a
/// search window.
struct DecodeHit {
    decoded: DecodedId,
    version: TrustmarkVersion,
    whole: bool,
}

/// Plausibility gate for a decoded id. TrustMark's decoder always emits 100 bits
/// and BCH will "correct" unwatermarked content into a *consistent* bogus id, so
/// a raw decode is not proof of a watermark. Constraining the result to the id
/// space we actually embed (a specific codec, a sane digit count) drops the
/// chance of a clean page being read as watermarked to negligible — and makes
/// even a single corroborated decode trustworthy enough to surface.
fn accept_decoded(decoded: &DecodedId, options: &PdfwmOptions) -> bool {
    if let IdCodecSelection::Specific(expected) = options.id_codec {
        if decoded.codec != expected {
            return false;
        }
    }
    if let Some(max_digits) = options.id_max_digits {
        if matches!(decoded.codec, IdCodec::UintDecimal | IdCodec::DecimalBcd)
            && decoded.id.len() > max_digits
        {
            return false;
        }
    }
    true
}

/// Decode an id from `image` after shrinking it to `DECODE_WORK_SIDE` (a no-op
/// to the result — TrustMark resizes to 256 regardless — but a large speed win),
/// returning it only if it passes the plausibility gate.
fn decode_one(
    tm: &Trustmark,
    image: &DynamicImage,
    options: &PdfwmOptions,
) -> Option<(DecodedId, TrustmarkVersion)> {
    let small = if image.width().max(image.height()) > DECODE_WORK_SIDE {
        image.resize(
            DECODE_WORK_SIDE,
            DECODE_WORK_SIDE,
            imageops::FilterType::Triangle,
        )
    } else {
        image.clone()
    };
    let bits = tm.decode(small).ok()?;
    let decoded = decode_id(&bits).ok()?;
    if !accept_decoded(&decoded, options) {
        return None;
    }
    let version = version_from_data_bits(bits.len()).ok()?;
    Some((decoded, version))
}

/// Decode an id from `image`, robust to the image being only a *crop* of a
/// tiled page. Tries the whole image first (legacy single-watermark documents
/// and full-page shots), then sweeps multi-scale sliding windows so any region
/// containing a whole tile is found and corroborated. The sweep stops as soon as
/// any id is seen by `min_search_votes` windows, so the common cases return after
/// only a handful of decodes.
fn decode_search(tm: &Trustmark, image: &DynamicImage, options: &PdfwmOptions) -> Vec<DecodeHit> {
    let mut hits = Vec::new();

    if let Some((decoded, version)) = decode_one(tm, image, options) {
        hits.push(DecodeHit {
            decoded,
            version,
            whole: true,
        });
    }

    if !options.search_decode {
        return hits;
    }

    let debug = std::env::var("PDFWM_DEBUG_VOTES").is_ok();
    let (width, height) = image.dimensions();
    let short = width.min(height);
    let mut scanned = 0usize;
    let mut decoded_windows = 0usize;
    let mut counts: HashMap<(String, IdCodec, TrustmarkVersion), usize> = HashMap::new();
    'scales: for &frac in SEARCH_FRACTIONS {
        let side = (short as f32 * frac).round() as u32;
        if side < MIN_WINDOW {
            continue;
        }
        let stride = ((side as f32 * (1.0 - SEARCH_OVERLAP)).round() as u32).max(1);
        for y in window_positions(height, side, stride) {
            for x in window_positions(width, side, stride) {
                if scanned >= options.max_search_windows {
                    break 'scales;
                }
                scanned += 1;
                let crop = image.crop_imm(x, y, side, side);
                if let Some((decoded, version)) = decode_one(tm, &crop, options) {
                    decoded_windows += 1;
                    let count = counts
                        .entry((decoded.id.clone(), decoded.codec, version))
                        .or_insert(0);
                    *count += 1;
                    let reached = *count >= options.min_search_votes;
                    hits.push(DecodeHit {
                        decoded,
                        version,
                        whole: false,
                    });
                    if reached && !debug {
                        break 'scales;
                    }
                }
            }
        }
    }

    if debug {
        let mut ranked: Vec<_> = counts.iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(a.1));
        eprintln!(
            "[pdfwm votes] scanned={scanned} decoded={decoded_windows} distinct={}",
            ranked.len()
        );
        for ((id, _, _), count) in ranked.iter().take(6) {
            eprintln!("   {count:3}  {id}");
        }
    }

    hits
}

/// Window start offsets covering `[0, total)` with the given side and stride,
/// always including the final flush-right position so the trailing edge is
/// scanned.
fn window_positions(total: u32, side: u32, stride: u32) -> Vec<u32> {
    if side >= total {
        return vec![0];
    }
    let max_start = total - side;
    let mut positions = Vec::new();
    let mut start = 0u32;
    loop {
        positions.push(start);
        if start >= max_start {
            break;
        }
        start = (start + stride).min(max_start);
    }

    // Keep at most MAX_POS_PER_AXIS, evenly spread and always including both
    // ends, so a single scale can't starve the others under the window cap.
    let max = MAX_POS_PER_AXIS as usize;
    if positions.len() > max {
        let last = positions.len() - 1;
        let mut spread = Vec::with_capacity(max);
        for k in 0..max {
            spread.push(positions[k * last / (max - 1)]);
        }
        spread.dedup();
        positions = spread;
    }
    positions
}

/// Resolve the decode hits to a single id, requiring `min_search_votes`
/// agreeing windows before trusting a search result. A lone whole-image decode
/// is still accepted as a backward-compatible (lower-confidence) fallback.
///
/// Returns `(id, version, votes, confidence)`.
fn decide_from_hits(
    hits: &[DecodeHit],
    min_search_votes: usize,
) -> Result<(DecodedId, TrustmarkVersion, usize, f64), PdfwmError> {
    if hits.is_empty() {
        return Err(PdfwmError::Watermark(
            "no TrustMark watermark could be decoded".to_string(),
        ));
    }

    // window votes + whether the whole image decoded, per (id, codec, version).
    let mut tally: HashMap<(String, IdCodec, TrustmarkVersion), (usize, bool)> = HashMap::new();
    for hit in hits {
        let entry = tally
            .entry((hit.decoded.id.clone(), hit.decoded.codec, hit.version))
            .or_insert((0, false));
        if hit.whole {
            entry.1 = true;
        } else {
            entry.0 += 1;
        }
    }

    let mut ranked: Vec<_> = tally.into_iter().collect();
    ranked.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)));

    // Strong path: an id corroborated by enough independent windows.
    let strong: Vec<_> = ranked
        .iter()
        .filter(|(_, (votes, _))| *votes >= min_search_votes)
        .collect();
    if let Some(((id, codec, version), (votes, _))) = strong.first().copied() {
        if strong.len() > 1 {
            let (_, (second_votes, _)) = strong[1];
            if *second_votes == *votes {
                return Err(PdfwmError::AmbiguousWatermark(
                    "multiple watermark ids decoded with equal support".to_string(),
                ));
            }
        }
        return Ok((
            DecodedId {
                id: id.clone(),
                codec: *codec,
            },
            *version,
            *votes,
            1.0,
        ));
    }

    // No id cleared the strong threshold. Every remaining hit still passed the
    // plausibility gate, so the best-supported one is a genuine candidate (a
    // legacy single-watermark document, or a partial screenshot that contained
    // only one tile). Surface it at low confidence so the operator can weigh it.
    let ((id, codec, version), (window_votes, whole)) = &ranked[0];
    let votes = (*window_votes).max(usize::from(*whole));
    Ok((
        DecodedId {
            id: id.clone(),
            codec: *codec,
        },
        *version,
        votes,
        0.5,
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_count_rounds_and_caps() {
        assert_eq!(tile_count(4250, 1280), 3); // US-Letter width @500dpi -> 3 cols
        assert_eq!(tile_count(5500, 1280), 4); // US-Letter height @500dpi -> 4 rows
        assert_eq!(tile_count(600, 1280), 1); // small image -> a single tile
        assert_eq!(tile_count(0, 1280), 1);
        assert_eq!(tile_count(1000, 0), 1); // tiling disabled
        assert_eq!(tile_count(1_000_000, 1280), MAX_TILES_PER_AXIS); // capped
    }

    #[test]
    fn tile_bounds_partition_is_exact_and_monotonic() {
        let bounds = tile_bounds(4250, 3);
        assert_eq!(bounds.first(), Some(&0));
        assert_eq!(bounds.last(), Some(&4250));
        assert_eq!(bounds.len(), 4);
        assert!(bounds.windows(2).all(|w| w[1] > w[0]));
    }

    #[test]
    fn feather_weight_zero_at_borders_one_inside() {
        assert_eq!(feather_weight(0, 100, 10), 0.0);
        assert_eq!(feather_weight(99, 100, 10), 0.0);
        assert_eq!(feather_weight(10, 100, 10), 1.0);
        assert_eq!(feather_weight(50, 100, 10), 1.0);
        assert_eq!(feather_weight(5, 100, 10), 0.5);
        assert_eq!(feather_weight(7, 100, 0), 1.0); // margin 0 -> no feathering
    }

    #[test]
    fn window_positions_cover_both_ends_and_are_bounded() {
        let positions = window_positions(1700, 567, 283);
        assert_eq!(positions.first(), Some(&0));
        assert_eq!(*positions.last().unwrap(), 1700 - 567); // flush-right included
        assert!(positions.len() as u32 <= MAX_POS_PER_AXIS);
        // A window at least as large as the image yields one position at the origin.
        assert_eq!(window_positions(100, 200, 50), vec![0]);
    }

    #[test]
    fn window_positions_subsample_keeps_ends() {
        // A very fine stride would exceed the per-axis cap; ends must survive.
        let positions = window_positions(4000, 200, 60);
        assert!(positions.len() as u32 <= MAX_POS_PER_AXIS);
        assert_eq!(positions.first(), Some(&0));
        assert_eq!(*positions.last().unwrap(), 4000 - 200);
    }
}
