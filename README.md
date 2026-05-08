# pdfwm PHP extension

`pdfwm` is a Rust-based PHP extension for embedding screenshot-robust TrustMark
watermarks into images and image-only PDF output. Business code passes an
external string ID; extraction returns that same ID directly. The extension does
not expose `payload_hex` and does not require an application-side payload
mapping table.

## Build

```sh
cargo build --release
```

The built PHP extension is:

```text
target/release/libpdfwm.dylib   # macOS
target/release/libpdfwm.so      # Linux
```

Load it with PHP using your normal extension deployment path, or for a local
smoke check:

```sh
php -d extension=/absolute/path/to/target/debug/libpdfwm.dylib -r 'var_dump(function_exists("pdfwm_embed_image_path"));'
```

## Install With Homebrew

```sh
brew tap dickwu/tap
brew install dickwu/tap/document-embed
php -m | grep pdfwm
```

The Homebrew formula installs the extension for Homebrew `php@8.3` and writes:

```text
$(brew --prefix)/etc/php/8.3/conf.d/ext-pdfwm.ini
```

## Runtime Configuration

TrustMark model files must already exist on disk. The extension never downloads
models during a PHP request.

```sh
export PDFWM_MODEL_DIR=/opt/pdfwm/trustmark-models
export PDFIUM_DYNAMIC_LIB_PATH=/opt/pdfium/libpdfium.dylib
```

`PDFIUM_DYNAMIC_LIB_PATH` is needed for PDF embed/extract unless Pdfium is
available as a system library. It may point either at the dynamic library file
or at the directory containing the platform-named Pdfium library.

Supported options include `id_codec`, `model_dir`, `pdfium_lib_path`, `dpi`,
`strength`, `variant`, `version`, `image_format`, `jpeg_quality`,
`embed_metadata`, `max_pages`, `max_pixels_per_page`, and `max_id_bytes`.

## API

```php
pdfwm_embed_image_path(string $inputImagePath, string $id, string $outputImagePath, array $options = []): array;
pdfwm_extract_image_path(string $imagePath, array $options = []): array;
pdfwm_embed_pdf_path(string $inputPdfPath, string $id, string $outputPdfPath, array $options = []): array;
pdfwm_extract_pdf_path(string $pdfPath, array $options = []): array;
pdfwm_read_metadata(string $pdfPath): array;
```

The upstream Rust TrustMark crate currently returns decoded payload bits without
a detector confidence score. This extension returns `confidence => 1.0` for
successful decodes; PDF extraction still votes across pages by decoded ID.

## ID Capacity

The default TrustMark version is `BCH_5`, with 61 protected data bits. The first
2 bits are used as a codec header, so direct IDs are intentionally small:

| Codec | BCH_5 capacity |
| --- | ---: |
| `uint_decimal` | `0..576460752303423487` without leading zeroes |
| `decimal_bcd` | up to 13 digits, preserving leading zeroes |
| `base36` | up to 10 chars `[0-9A-Z]`, preserving leading zeroes |
| `utf8_raw` | up to 7 UTF-8 bytes |

`id_codec` defaults to `auto`: `uint_decimal`, then `decimal_bcd`, then
`base36`, then `utf8_raw`. IDs that do not fit fail before image/PDF processing
with `PdfwmIdTooLongException`.

## PDF Output

PDF embedding rasterizes every page, applies the pixel watermark, then rebuilds
an image-only PDF. The output PDF does not guarantee preservation of searchable
text, links, forms, bookmarks, attachments, layers, or vector structure. File
size can increase substantially. Higher DPI improves raster quality but makes
generation slower and larger.

TrustMark is not DRM, encryption, or malicious-removal-proof steganography. It
improves leak tracing through screenshots and common image transforms, but it
cannot stop a user from taking screenshots, photographing a screen, or attacking
the watermark.

## Release

Releases are tag-driven:

```sh
git tag v0.1.0
git push origin main v0.1.0
```

GitHub Actions verifies Linux and macOS builds, creates a GitHub release,
generates `Formula/document-embed.rb`, and pushes it to `dickwu/homebrew-tap`.
The release workflow requires the `HOMEBREW_TAP_DEPLOY_KEY` repository secret.
