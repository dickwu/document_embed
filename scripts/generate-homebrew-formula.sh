#!/usr/bin/env bash
set -euo pipefail

: "${VERSION:?VERSION is required}"
: "${URL:?URL is required}"
: "${SHA256:?SHA256 is required}"

cat <<RUBY
class DocumentEmbed < Formula
  desc "PHP extension for screenshot-robust PDF and image TrustMark IDs"
  homepage "https://github.com/dickwu/document_embed"
  url "${URL}"
  version "${VERSION}"
  sha256 "${SHA256}"
  license "MIT"

  depends_on "llvm" => :build
  depends_on "rust" => :build
  depends_on "php@8.3"

  def install
    php = Formula["php@8.3"]
    ENV["PHP"] = php.opt_bin/"php"
    ENV["PHP_CONFIG"] = php.opt_bin/"php-config"
    ENV["LIBCLANG_PATH"] = Formula["llvm"].opt_lib.to_s

    system "cargo", "build", "--jobs", ENV.make_jobs.to_s, "--lib", "--release", "--locked"

    extension = OS.mac? ? "dylib" : "so"
    (lib/"php/extensions").install "target/release/libpdfwm.#{extension}" => "pdfwm.so"

    (etc/"php/8.3/conf.d").mkpath
    (etc/"php/8.3/conf.d/ext-pdfwm.ini").write <<~INI
      extension=#{opt_lib}/php/extensions/pdfwm.so
    INI
  end

  def caveats
    <<~EOS
      The formula writes:
        #{etc}/php/8.3/conf.d/ext-pdfwm.ini

      TrustMark models are not bundled. Set PDFWM_MODEL_DIR to the directory
      containing encoder_Q.onnx and decoder_Q.onnx before embedding/extracting.

      PDF workflows also need PDFIUM_DYNAMIC_LIB_PATH pointing at libpdfium.

      Restart long-running PHP processes after install or upgrade.
    EOS
  end

  test do
    php = Formula["php@8.3"].opt_bin/"php"
    script = <<~PHP
      echo function_exists("pdfwm_embed_image_path") && class_exists("PdfwmConfigException") ? "OK" : "FAIL";
    PHP

    assert_equal "OK", shell_output("#{php} -n -d extension=#{lib}/php/extensions/pdfwm.so -r '#{script}'")
  end
end
RUBY
