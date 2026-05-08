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
  depends_on "php"

  def php_formula
    Formula["php"]
  end

  def php_major_minor
    version = Utils.safe_popen_read(php_formula.opt_bin/"php-config", "--version").strip
    match = version.match(/\A(\d+\.\d+)/)
    odie "Unable to determine PHP major/minor version from #{version.inspect}" if match.nil?
    match[1]
  end

  def php_conf_d
    etc/"php/#{php_major_minor}/conf.d"
  end

  def install
    ENV["PHP"] = php_formula.opt_bin/"php"
    ENV["PHP_CONFIG"] = php_formula.opt_bin/"php-config"
    ENV["LIBCLANG_PATH"] = Formula["llvm"].opt_lib.to_s

    system "cargo", "build", "--jobs", ENV.make_jobs.to_s, "--lib", "--release", "--locked"

    extension = OS.mac? ? "dylib" : "so"
    (lib/"php/extensions").install "target/release/libpdfwm.#{extension}" => "pdfwm.so"

    Pathname.glob(etc/"php/*/conf.d/ext-pdfwm.ini").each do |config|
      rm_f config unless config == php_conf_d/"ext-pdfwm.ini"
    end

    php_conf_d.mkpath
    (php_conf_d/"ext-pdfwm.ini").write <<~INI
      extension=#{opt_lib}/php/extensions/pdfwm.so
    INI
  end

  def caveats
    <<~EOS
      The formula writes:
        #{php_conf_d}/ext-pdfwm.ini

      TrustMark models are not bundled. Set PDFWM_MODEL_DIR to the directory
      containing encoder_Q.onnx and decoder_Q.onnx before embedding/extracting.

      PDF workflows also need PDFIUM_DYNAMIC_LIB_PATH pointing at libpdfium.

      Restart long-running PHP processes after install or upgrade.
    EOS
  end

  test do
    php = php_formula.opt_bin/"php"
    assert_path_exists php_conf_d/"ext-pdfwm.ini"
    assert_match "#{opt_lib}/php/extensions/pdfwm.so", (php_conf_d/"ext-pdfwm.ini").read

    script = <<~PHP
      echo function_exists("pdfwm_embed_image_path") && class_exists("PdfwmConfigException") ? "OK" : "FAIL";
    PHP

    assert_equal "OK", shell_output("#{php} -n -d extension=#{lib}/php/extensions/pdfwm.so -r '#{script}'")
    assert_match "pdfwm_php_ext", shell_output("#{php} -m")
  end
end
RUBY
