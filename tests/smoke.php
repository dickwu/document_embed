<?php

function fail(string $message): void
{
    fwrite(STDERR, $message . PHP_EOL);
    exit(1);
}

foreach ([
    'pdfwm_embed_image_path',
    'pdfwm_extract_image_path',
    'pdfwm_embed_pdf_path',
    'pdfwm_extract_pdf_path',
    'pdfwm_read_metadata',
] as $function) {
    if (!function_exists($function)) {
        fail("missing function: {$function}");
    }
}

foreach ([
    PdfwmInvalidArgumentException::class,
    PdfwmInvalidIdException::class,
    PdfwmIdTooLongException::class,
    PdfwmConfigException::class,
    PdfwmPdfException::class,
    PdfwmImageException::class,
    PdfwmWatermarkException::class,
    PdfwmAmbiguousWatermarkException::class,
    PdfwmLimitException::class,
] as $class) {
    if (!class_exists($class)) {
        fail("missing class: {$class}");
    }
}

try {
    pdfwm_extract_image_path(__FILE__);
    fail('expected missing model_dir to throw PdfwmConfigException');
} catch (PdfwmConfigException $e) {
    if (!str_contains($e->getMessage(), 'PDFWM_MODEL_DIR')) {
        fail('unexpected config exception message: ' . $e->getMessage());
    }
}

try {
    pdfwm_extract_image_path(__FILE__, ['id_codec' => 'bogus']);
    fail('expected invalid id_codec to throw PdfwmInvalidArgumentException');
} catch (PdfwmInvalidArgumentException $e) {
    if (!str_contains($e->getMessage(), 'id_codec')) {
        fail('unexpected invalid argument exception message: ' . $e->getMessage());
    }
}

echo "OK\n";
