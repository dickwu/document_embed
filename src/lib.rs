#![cfg_attr(windows, feature(abi_vectorcall))]

use engine::{EmbedImageResult, EmbedPdfResult, ExtractImageResult, ExtractPdfResult};
use error::PdfwmError;
use ext_php_rs::{
    boxed::ZBox,
    exception::{PhpException, PhpResult},
    prelude::*,
    types::ZendHashTable,
    zend::ce,
};

pub mod engine;
pub mod error;
pub mod id_codec;
pub mod metadata;
pub mod options;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmInvalidArgumentException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmInvalidIdException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmIdTooLongException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmConfigException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmPdfException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmImageException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmWatermarkException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmAmbiguousWatermarkException;

#[php_class]
#[php(extends(ce = ce::exception, stub = "\\Exception"))]
#[derive(Default)]
pub struct PdfwmLimitException;

#[php_function]
pub fn pdfwm_embed_image_path(
    input_image_path: String,
    id: String,
    output_image_path: String,
    options: Option<&ZendHashTable>,
) -> PhpResult<ZBox<ZendHashTable>> {
    let options = options::PdfwmOptions::from_php(options).map_err(to_php_exception)?;
    let result = engine::embed_image_path(&input_image_path, &id, &output_image_path, &options)
        .map_err(to_php_exception)?;
    embed_image_result_to_array(result).map_err(to_php_exception)
}

#[php_function]
pub fn pdfwm_extract_image_path(
    image_path: String,
    options: Option<&ZendHashTable>,
) -> PhpResult<ZBox<ZendHashTable>> {
    let options = options::PdfwmOptions::from_php(options).map_err(to_php_exception)?;
    let result = engine::extract_image_path(&image_path, &options).map_err(to_php_exception)?;
    extract_image_result_to_array(result).map_err(to_php_exception)
}

#[php_function]
pub fn pdfwm_embed_pdf_path(
    input_pdf_path: String,
    id: String,
    output_pdf_path: String,
    options: Option<&ZendHashTable>,
) -> PhpResult<ZBox<ZendHashTable>> {
    let options = options::PdfwmOptions::from_php(options).map_err(to_php_exception)?;
    let result = engine::embed_pdf_path(&input_pdf_path, &id, &output_pdf_path, &options)
        .map_err(to_php_exception)?;
    embed_pdf_result_to_array(result).map_err(to_php_exception)
}

#[php_function]
pub fn pdfwm_extract_pdf_path(
    pdf_path: String,
    options: Option<&ZendHashTable>,
) -> PhpResult<ZBox<ZendHashTable>> {
    let options = options::PdfwmOptions::from_php(options).map_err(to_php_exception)?;
    let result = engine::extract_pdf_path(&pdf_path, &options).map_err(to_php_exception)?;
    extract_pdf_result_to_array(result).map_err(to_php_exception)
}

#[php_function]
pub fn pdfwm_read_metadata(pdf_path: String) -> PhpResult<ZBox<ZendHashTable>> {
    metadata::read_pdf_metadata(&pdf_path).map_err(to_php_exception)
}

#[php_module]
pub fn get_module(module: ModuleBuilder) -> ModuleBuilder {
    module
        .class::<PdfwmInvalidArgumentException>()
        .class::<PdfwmInvalidIdException>()
        .class::<PdfwmIdTooLongException>()
        .class::<PdfwmConfigException>()
        .class::<PdfwmPdfException>()
        .class::<PdfwmImageException>()
        .class::<PdfwmWatermarkException>()
        .class::<PdfwmAmbiguousWatermarkException>()
        .class::<PdfwmLimitException>()
        .function(wrap_function!(pdfwm_embed_pdf_path))
        .function(wrap_function!(pdfwm_embed_image_path))
        .function(wrap_function!(pdfwm_extract_image_path))
        .function(wrap_function!(pdfwm_extract_pdf_path))
        .function(wrap_function!(pdfwm_read_metadata))
}

fn embed_image_result_to_array(
    result: EmbedImageResult,
) -> Result<ZBox<ZendHashTable>, PdfwmError> {
    let mut output = ZendHashTable::new();
    output.insert("ok", true).map_err(array_error)?;
    output
        .insert("input_path", result.input_path)
        .map_err(array_error)?;
    output
        .insert("output_path", result.output_path)
        .map_err(array_error)?;
    output.insert("id", result.id).map_err(array_error)?;
    output
        .insert("id_codec", result.id_codec.to_string())
        .map_err(array_error)?;
    output
        .insert("version", result.version.to_string())
        .map_err(array_error)?;
    output
        .insert("variant", result.variant.to_string())
        .map_err(array_error)?;
    output
        .insert("data_bits", result.data_bits as i64)
        .map_err(array_error)?;
    Ok(output)
}

fn embed_pdf_result_to_array(result: EmbedPdfResult) -> Result<ZBox<ZendHashTable>, PdfwmError> {
    let mut output = embed_image_result_to_array(EmbedImageResult {
        input_path: result.input_path,
        output_path: result.output_path,
        id: result.id,
        id_codec: result.id_codec,
        version: result.version,
        variant: result.variant,
        data_bits: result.data_bits,
    })?;
    output
        .insert("page_count", result.page_count as i64)
        .map_err(array_error)?;
    output
        .insert("image_only_pdf", result.image_only_pdf)
        .map_err(array_error)?;
    Ok(output)
}

fn extract_image_result_to_array(
    result: ExtractImageResult,
) -> Result<ZBox<ZendHashTable>, PdfwmError> {
    let mut output = ZendHashTable::new();
    output.insert("ok", true).map_err(array_error)?;
    output.insert("id", result.id).map_err(array_error)?;
    output
        .insert("id_codec", result.id_codec.to_string())
        .map_err(array_error)?;
    output
        .insert("version", result.version.to_string())
        .map_err(array_error)?;
    output
        .insert("variant", result.variant.to_string())
        .map_err(array_error)?;
    output
        .insert("confidence", result.confidence)
        .map_err(array_error)?;
    output
        .insert("votes", result.votes as i64)
        .map_err(array_error)?;
    Ok(output)
}

fn extract_pdf_result_to_array(
    result: ExtractPdfResult,
) -> Result<ZBox<ZendHashTable>, PdfwmError> {
    let mut output = extract_image_result_to_array(ExtractImageResult {
        id: result.id,
        id_codec: result.id_codec,
        version: result.version,
        variant: result.variant,
        confidence: 1.0,
        votes: result.votes,
    })?;
    output
        .insert("page_count", result.page_count as i64)
        .map_err(array_error)?;
    output
        .insert("votes", result.votes as i64)
        .map_err(array_error)?;

    let mut pages = ZendHashTable::new();
    for page_result in result.page_results {
        let mut row = ZendHashTable::new();
        row.insert("page", page_result.page as i64)
            .map_err(array_error)?;
        row.insert("ok", page_result.ok).map_err(array_error)?;
        if let Some(id) = page_result.id {
            row.insert("id", id).map_err(array_error)?;
        }
        if let Some(codec) = page_result.id_codec {
            row.insert("id_codec", codec.to_string())
                .map_err(array_error)?;
        }
        if let Some(confidence) = page_result.confidence {
            row.insert("confidence", confidence).map_err(array_error)?;
        }
        if let Some(error) = page_result.error {
            row.insert("error", error).map_err(array_error)?;
        }
        pages.push(row).map_err(array_error)?;
    }

    output.insert("page_results", pages).map_err(array_error)?;
    Ok(output)
}

fn array_error(err: ext_php_rs::error::Error) -> PdfwmError {
    PdfwmError::InvalidArgument(format!("failed to build PHP return array: {err}"))
}

fn to_php_exception(error: PdfwmError) -> PhpException {
    let message = error.to_string();
    match error {
        PdfwmError::InvalidArgument(_) => {
            PhpException::from_class::<PdfwmInvalidArgumentException>(message)
        }
        PdfwmError::InvalidId(_) => PhpException::from_class::<PdfwmInvalidIdException>(message),
        PdfwmError::IdTooLong(_) => PhpException::from_class::<PdfwmIdTooLongException>(message),
        PdfwmError::Config(_) => PhpException::from_class::<PdfwmConfigException>(message),
        PdfwmError::Pdf(_) => PhpException::from_class::<PdfwmPdfException>(message),
        PdfwmError::Image(_) => PhpException::from_class::<PdfwmImageException>(message),
        PdfwmError::Watermark(_) => PhpException::from_class::<PdfwmWatermarkException>(message),
        PdfwmError::AmbiguousWatermark(_) => {
            PhpException::from_class::<PdfwmAmbiguousWatermarkException>(message)
        }
        PdfwmError::Limit(_) => PhpException::from_class::<PdfwmLimitException>(message),
    }
}
