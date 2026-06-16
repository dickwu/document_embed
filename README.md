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
$(brew --prefix php)/bin/php -m | grep pdfwm
```

The Homebrew formula builds the extension against Homebrew `php` using that
formula's `php-config`, then writes an ini file for the detected PHP
major/minor version. For example, with Homebrew PHP 8.5 it writes:

```text
$(brew --prefix)/etc/php/8.5/conf.d/ext-pdfwm.ini
```

If your shell still resolves `php` to a versioned formula such as `php@8.3`,
use `$(brew --prefix php)/bin/php` or update your `PATH` so the Homebrew `php`
formula is first.

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

Tiling and partial-screenshot decoding add:

| Option | Default | Purpose |
| --- | --- | --- |
| `tile_size` | `1280` | Target tile side (px) of the watermark grid; `0` disables tiling (single whole-image watermark). |
| `tile_feather` | auto | Residual edge-feather width (px) inside each tile; `0` derives it from the tile size. |
| `search_decode` | `true` | Run the sliding-window search on extract (needed to decode tiled output and partial screenshots). |
| `min_search_votes` | `2` | Agreeing windows required for a high-confidence decode (a single decode is still returned at low confidence). |
| `max_search_windows` | `180` | Upper bound on windows scanned per image. |
| `id_max_digits` | unset | Reject decoded numeric ids longer than this — paired with an `id_codec` constraint it discards noise decodes from unwatermarked regions. |

## API

```php
pdfwm_embed_image_path(string $inputImagePath, string $id, string $outputImagePath, array $options = []): array;
pdfwm_extract_image_path(string $imagePath, array $options = []): array;
pdfwm_embed_pdf_path(string $inputPdfPath, string $id, string $outputPdfPath, array $options = []): array;
pdfwm_extract_pdf_path(string $pdfPath, array $options = []): array;
pdfwm_read_metadata(string $pdfPath): array;
```

Extraction returns `id`, `confidence` (1.0 when corroborated by
`min_search_votes` windows, lower for a single decode), and `votes` (how many
windows agreed). PDF extraction additionally votes across pages.

### Tiled watermark & partial-screenshot tracing

A single whole-image TrustMark is keyed to the entire frame, so a *cropped*
screenshot of a watermarked page cannot be decoded, and its residual — a 256×256
pattern stretched across the whole page — shows up as a faint low-frequency
"shadow" on white backgrounds. To fix both, embedding splits each page into a
grid of ~square tiles (`tile_size`) and watermarks each tile with the same id:

- A partial screenshot that contains one whole tile is still traceable.
- Each tile's residual is upscaled only ~`tile/256`×, so the shadow becomes fine
  and far less visible; `strength` scales it down further.
- The residual is feathered to zero at tile borders, so the grid is seamless.

Extraction mirrors this: it tries the whole image (legacy single-watermark
output and full-page shots) and then sweeps multi-scale sliding windows, voting
across whatever regions decode. Because TrustMark's decoder always emits bits
and BCH will "correct" even unwatermarked content into a *consistent* bogus id,
votes alone are not proof of a watermark — constrain results to the id space you
actually embed (`id_codec` + `id_max_digits`) so a clean page cannot be misread.

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

Pages are embedded at the **full rasterized resolution** (no silent
downsampling) and stored as high-quality JPEG (`jpeg_quality`, default 92). The
default `dpi` is **500**, which keeps form text crisp; lower it (e.g. `dpi`/
`PDFWM_DPI`) to trade sharpness for smaller, faster output. A 500 DPI US-Letter
page is roughly 23 MP and ~1–2 MB per page after JPEG compression.

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
