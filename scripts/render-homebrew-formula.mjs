#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const [, , manifestPathArg, binaryUrl, outputPathArg] = process.argv;

if (!manifestPathArg || !binaryUrl || !outputPathArg) {
  console.error(
    "Usage: scripts/render-homebrew-formula.mjs <release-manifest.json> <binary-url> <output-path>",
  );
  process.exit(1);
}

const manifestPath = path.resolve(manifestPathArg);
const outputPath = path.resolve(outputPathArg);
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));

const binaryName = manifest.binary_name;
const checksum = manifest.checksum_sha256;
const version = manifest.version;

if (!binaryName || !checksum || !version) {
  console.error("release manifest must include binary_name, checksum_sha256, and version");
  process.exit(1);
}

const formula = `class Opengpu < Formula
  desc "MundusX contributor CLI"
  homepage "https://github.com/mundusx/mundusx"
  url "${binaryUrl}"
  version "${version}"
  sha256 "${checksum}"

  def install
    bin.install "${binaryName}" => "opengpu"
    bin.install_symlink "opengpu" => "mundusx"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/opengpu --version")
    assert_match version.to_s, shell_output("#{bin}/mundusx --version")
  end
end
`;

fs.mkdirSync(path.dirname(outputPath), { recursive: true });
fs.writeFileSync(outputPath, formula);
