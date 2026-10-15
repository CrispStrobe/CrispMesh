// Test-only adapter for pinned libELAS. This file is not linked into CrispMesh.
#include "elas.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
namespace fs = std::filesystem;
struct Image {
  int width = 0;
  int height = 0;
  std::vector<std::uint8_t> pixels;
};

Image read_pgm(const std::string& path) {
  std::ifstream in(path, std::ios::binary);
  if (!in) throw std::runtime_error("Cannot open PGM: " + path);
  std::string magic;
  Image image;
  int max_value = 0;
  in >> magic >> image.width >> image.height >> max_value;
  if (!in || magic != "P5" || image.width < 64 || image.height < 32 ||
      static_cast<std::uint64_t>(image.width) * image.height > 4000000ULL ||
      max_value != 255 || in.get() != '\n')
    throw std::runtime_error("Invalid or oversized P5 PGM: " + path);
  image.pixels.resize(static_cast<std::size_t>(image.width) * image.height);
  in.read(reinterpret_cast<char*>(image.pixels.data()), static_cast<std::streamsize>(image.pixels.size()));
  if (!in) throw std::runtime_error("Truncated PGM: " + path);
  return image;
}

void write_pfm(const std::string& path, const std::vector<float>& disparity, int width, int height) {
  std::ofstream out(path, std::ios::binary);
  if (!out) throw std::runtime_error("Cannot write PFM: " + path);
  // Built as x86_64 on macOS: little-endian IEEE 754 floats.
  out << "Pf\n" << width << ' ' << height << "\n-1.0\n";
  for (int y = height - 1; y >= 0; --y)
    out.write(reinterpret_cast<const char*>(disparity.data() + static_cast<std::size_t>(y) * width),
              static_cast<std::streamsize>(width * sizeof(float)));
  if (!out) throw std::runtime_error("Failed writing PFM: " + path);
}

int integer(const std::string& text) {
  std::size_t used = 0;
  int value = std::stoi(text, &used);
  if (used != text.size()) throw std::runtime_error("Invalid ndisp");
  return value;
}
} // namespace

int main(int argc, char** argv) {
  try {
    std::map<std::string, std::string> args;
    for (int i = 1; i < argc; i += 2) {
      if (i + 1 >= argc || std::string(argv[i]).rfind("--", 0) != 0)
        throw std::runtime_error("Expected --left PGM --right PGM --ndisp COUNT --output PFM");
      const std::string name = argv[i] + 2;
      if (name != "left" && name != "right" && name != "ndisp" && name != "output")
        throw std::runtime_error("Unknown argument: " + name);
      if (!args.emplace(name, argv[i + 1]).second) throw std::runtime_error("Duplicate argument: " + name);
    }
    if (args.size() != 4) throw std::runtime_error("Expected --left PGM --right PGM --ndisp COUNT --output PFM");
    const Image left = read_pgm(args.at("left"));
    const Image right = read_pgm(args.at("right"));
    if (left.width != right.width || left.height != right.height)
      throw std::runtime_error("Stereo PGM dimensions differ");
    const int ndisp = integer(args.at("ndisp"));
    if (ndisp < 16 || ndisp > 2048 || ndisp >= left.width)
      throw std::runtime_error("ndisp must be 16..2048 and less than image width");
    if (static_cast<std::uint64_t>(left.width) * left.height * ndisp > 600000000ULL)
      throw std::runtime_error("Stereo image and disparity range exceed work budget");
    if (fs::exists(fs::symlink_status(args.at("output"))))
      throw std::runtime_error("Output already exists");
    Elas::parameters settings(Elas::MIDDLEBURY);
    settings.disp_min = 0;
    settings.disp_max = ndisp - 1; // Inclusive ELAS maximum; manifest ndisp is count.
    settings.subsampling = false;
    Elas elas(settings);
    const int32_t dims[3] = {left.width, left.height, left.width};
    std::vector<float> disparity(left.pixels.size(), std::numeric_limits<float>::infinity());
    std::vector<float> right_disparity(right.pixels.size(), std::numeric_limits<float>::infinity());
    elas.process(const_cast<std::uint8_t*>(left.pixels.data()),
                 const_cast<std::uint8_t*>(right.pixels.data()),
                 disparity.data(), right_disparity.data(), dims);
    std::size_t valid = 0;
    for (float& value : disparity) {
      if (!std::isfinite(value) || value < 0 || value >= ndisp)
        value = std::numeric_limits<float>::infinity();
      else ++valid;
    }
    if (valid == 0) throw std::runtime_error("ELAS produced no valid disparities");
    write_pfm(args.at("output"), disparity, left.width, left.height);
    std::cout << "{\"oracle\":\"libELAS\",\"preset\":\"MIDDLEBURY\",\"disp_min\":0,\"disp_max_inclusive\":"
              << ndisp - 1 << ",\"valid\":" << valid << ",\"total\":" << disparity.size() << "}\n";
    return 0;
  } catch (const std::exception& e) {
    std::cerr << "libELAS oracle: " << e.what() << '\n';
    return 1;
  }
}
