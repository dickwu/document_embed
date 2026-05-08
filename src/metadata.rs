use chrono::Utc;
use ext_php_rs::{boxed::ZBox, types::ZendHashTable};
use lopdf::{Dictionary, Document, Object};

use crate::{
    error::PdfwmError,
    id_codec::{IdCodec, TrustmarkVersion},
};

pub fn write_pdf_metadata(
    pdf_path: &str,
    id: &str,
    id_codec: IdCodec,
    version: TrustmarkVersion,
    variant: trustmark::Variant,
) -> Result<(), PdfwmError> {
    let mut document = Document::load(pdf_path)
        .map_err(|err| PdfwmError::Pdf(format!("failed to reopen output PDF metadata: {err}")))?;

    let info_id = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|object| object.as_reference().ok())
        .unwrap_or_else(|| {
            let id = document.add_object(Dictionary::new());
            document.trailer.set("Info", Object::Reference(id));
            id
        });

    let info = document
        .get_object_mut(info_id)
        .and_then(Object::as_dict_mut)
        .map_err(|err| PdfwmError::Pdf(format!("failed to write output PDF metadata: {err}")))?;

    info.set("PdfwmId", Object::string_literal(id));
    info.set("PdfwmIdCodec", Object::string_literal(id_codec.to_string()));
    info.set("PdfwmVersion", Object::string_literal(version.to_string()));
    info.set("PdfwmVariant", Object::string_literal(variant.to_string()));
    info.set(
        "PdfwmCreatedAt",
        Object::string_literal(Utc::now().to_rfc3339()),
    );

    document
        .save(pdf_path)
        .map_err(|err| PdfwmError::Pdf(format!("failed to save output PDF metadata: {err}")))?;
    Ok(())
}

pub fn read_pdf_metadata(pdf_path: &str) -> Result<ZBox<ZendHashTable>, PdfwmError> {
    let document = Document::load(pdf_path)
        .map_err(|err| PdfwmError::Pdf(format!("failed to open PDF metadata: {err}")))?;
    let mut output = ZendHashTable::new();
    output.insert("ok", true).map_err(to_pdf_error)?;

    let Some(info_id) = document
        .trailer
        .get(b"Info")
        .ok()
        .and_then(|object| object.as_reference().ok())
    else {
        return Ok(output);
    };

    let info = document
        .get_object(info_id)
        .and_then(Object::as_dict)
        .map_err(|err| PdfwmError::Pdf(format!("failed to read PDF metadata: {err}")))?;

    for (pdf_key, php_key) in [
        (b"PdfwmId".as_slice(), "id"),
        (b"PdfwmIdCodec".as_slice(), "id_codec"),
        (b"PdfwmVersion".as_slice(), "version"),
        (b"PdfwmVariant".as_slice(), "variant"),
        (b"PdfwmCreatedAt".as_slice(), "created_at"),
    ] {
        if let Ok(value) = info.get(pdf_key).and_then(Object::as_str) {
            output
                .insert(php_key, String::from_utf8_lossy(value).to_string())
                .map_err(to_pdf_error)?;
        }
    }

    Ok(output)
}

fn to_pdf_error(err: ext_php_rs::error::Error) -> PdfwmError {
    PdfwmError::Pdf(format!("failed to build PHP array: {err}"))
}
