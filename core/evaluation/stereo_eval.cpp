#include "stereo_eval.hpp"

#include <opencv2/calib3d.hpp>

#include <algorithm>
#include <chrono>
#include <bit>
#include <charconv>
#include <cmath>
#include <cstdint>
#include <fstream>
#include <limits>
#include <regex>
#include <stdexcept>
#include <string>
#include <vector>

namespace crispmesh::evaluation {
namespace {
constexpr std::size_t kMaxPixels = 100'000'000;

void check_size(int width, int height) {
  if (width <= 0 || height <= 0 ||
      static_cast<std::uint64_t>(width) * height > kMaxPixels)
    throw std::runtime_error("Invalid or oversized image dimensions");
}

double number(const std::string& s, const char* name) {
  std::size_t used = 0;
  double value = std::stod(s, &used);
  if (used != s.size() || !std::isfinite(value))
    throw std::runtime_error(std::string("Invalid calibration ") + name);
  return value;
}

std::string field(const std::string& text, const std::string& key) {
  std::regex re("(?:^|\\n)" + key + "[ \\t]*=[ \\t]*([^\\r\\n]+)");
  std::smatch match;
  if (!std::regex_search(text, match, re))
    throw std::runtime_error("Missing calibration field: " + key);
  std::string value = match[1];
  value.erase(value.find_last_not_of(" \t") + 1);
  return value;
}

std::uint32_t swap32(std::uint32_t n) {
  return ((n & 0xff000000u) >> 24) | ((n & 0x00ff0000u) >> 8) |
         ((n & 0x0000ff00u) << 8) | ((n & 0x000000ffu) << 24);
}
} // namespace

Calibration read_middlebury_calibration(const std::filesystem::path& path) {
  std::ifstream in(path);
  if (!in) throw std::runtime_error("Cannot open calibration: " + path.string());
  if (std::filesystem::file_size(path) > 64 * 1024) throw std::runtime_error("Oversized calibration file");
  std::string text((std::istreambuf_iterator<char>(in)), {});
  const auto cam = field(text, "cam0");
  std::regex matrix(R"(^\[\s*([^\s,;]+)[\s,]+[^;]+;[^;]+;[^\]]+\]$)");
  std::smatch match;
  if (!std::regex_match(cam, match, matrix))
    throw std::runtime_error("Invalid cam0 matrix");
  Calibration c;
  c.fx = number(match[1], "cam0 fx");
  c.baseline = number(field(text, "baseline"), "baseline");
  c.doffs = number(field(text, "doffs"), "doffs");
  auto bounded_int = [&](const char* key, int max) {
    double value = number(field(text, key), key);
    if (value < 1 || value > max || std::floor(value) != value)
      throw std::runtime_error(std::string("Invalid calibration ") + key);
    return static_cast<int>(value);
  };
  c.ndisp = bounded_int("ndisp", 2048);
  c.width = bounded_int("width", 100'000);
  c.height = bounded_int("height", 100'000);
  if (c.fx <= 0 || c.baseline <= 0 || c.ndisp <= 0) throw std::runtime_error("Invalid calibration values");
  check_size(c.width, c.height);
  return c;
}

cv::Mat read_pfm(const std::filesystem::path& path) {
  std::ifstream in(path, std::ios::binary);
  if (!in) throw std::runtime_error("Cannot open PFM: " + path.string());
  auto line = [&]() {
    std::string value;
    for (int i = 0; i < 256; ++i) {
      const int c = in.get();
      if (c == '\n') return value;
      if (c == EOF) throw std::runtime_error("Truncated PFM header");
      value.push_back(static_cast<char>(c));
    }
    throw std::runtime_error("Oversized PFM header line");
  };
  const std::string magic = line(), dimensions = line(), scale_line = line();
  if (magic != "Pf") throw std::runtime_error("Expected one-channel Pf image");
  const auto separator = dimensions.find(' ');
  if (separator == std::string::npos) throw std::runtime_error("Invalid PFM dimensions");
  auto parse_dim = [](const std::string& value) {
    int result = 0;
    const char* first = value.data();
    const char* last = first + value.size();
    auto [end, error] = std::from_chars(first, last, result);
    if (error != std::errc{} || end != last) throw std::runtime_error("Invalid PFM dimensions");
    return result;
  };
  const int width = parse_dim(dimensions.substr(0, separator));
  const int height = parse_dim(dimensions.substr(separator + 1));
  check_size(width, height);
  const double scale = number(scale_line, "PFM scale");
  if (scale == 0) throw std::runtime_error("Zero PFM scale");
  cv::Mat result(height, width, CV_32F);
  std::vector<std::uint32_t> row(static_cast<std::size_t>(width));
  const bool swap = (scale < 0) != (std::endian::native == std::endian::little);
  for (int y = height - 1; y >= 0; --y) {
    in.read(reinterpret_cast<char*>(row.data()), static_cast<std::streamsize>(row.size() * 4));
    if (!in) throw std::runtime_error("Truncated PFM payload");
    for (int x = 0; x < width; ++x) {
      float raw = std::bit_cast<float>(swap ? swap32(row[x]) : row[x]);
      // Middlebury's PFM convention uses the sign for byte order and ignores
      // magnitude; raw finite values are disparity in pixels.
      result.at<float>(y, x) = raw;
    }
  }
  return result;
}

void write_pfm(const std::filesystem::path& path, const cv::Mat& image) {
  if (image.empty() || image.type() != CV_32F) throw std::runtime_error("PFM output requires CV_32F");
  check_size(image.cols, image.rows);
  std::ofstream out(path, std::ios::binary);
  if (!out) throw std::runtime_error("Cannot write PFM: " + path.string());
  out << "Pf\n" << image.cols << ' ' << image.rows << '\n'
      << (std::endian::native == std::endian::little ? "-1.0\n" : "1.0\n");
  for (int y = image.rows - 1; y >= 0; --y)
    out.write(reinterpret_cast<const char*>(image.ptr<float>(y)), static_cast<std::streamsize>(image.cols * 4));
  if (!out) throw std::runtime_error("Failed writing PFM payload");
}

Metrics evaluate_disparity(const cv::Mat& prediction, const cv::Mat& truth, const cv::Mat& mask) {
  if (prediction.empty() || truth.empty() || prediction.type() != CV_32F || truth.type() != CV_32F ||
      prediction.size() != truth.size() || (!mask.empty() && (mask.type() != CV_8U || mask.size() != truth.size())))
    throw std::runtime_error("Prediction, truth, and mask must have matching dimensions and types");
  Metrics m;
  double abs_sum = 0, squared_sum = 0;
  for (int y = 0; y < truth.rows; ++y) for (int x = 0; x < truth.cols; ++x) {
    if (!mask.empty() && mask.at<std::uint8_t>(y, x) == 0) continue;
    ++m.roi_pixels;
    const float gt = truth.at<float>(y, x);
    if (!std::isfinite(gt) || gt < 0) continue;
    ++m.gt_valid;
    const float pred = prediction.at<float>(y, x);
    if (!std::isfinite(pred) || pred < 0) continue;
    ++m.matched;
    const double err = std::abs(static_cast<double>(pred) - gt);
    abs_sum += err;
    squared_sum += err * err;
    if (err > 2.0) ++m.bad2_matched;
  }
  if (m.gt_valid) {
    m.coverage = static_cast<double>(m.matched) / m.gt_valid;
    m.bad2_all_valid = static_cast<double>(m.gt_valid - m.matched + m.bad2_matched) / m.gt_valid;
  }
  if (m.matched) {
    m.mean_abs_error_matched = abs_sum / m.matched;
    m.rms_error_matched = std::sqrt(squared_sum / m.matched);
    m.bad2_matched_rate = static_cast<double>(m.bad2_matched) / m.matched;
  }
  return m;
}

cv::Mat compute_sgbm(const cv::Mat& left, const cv::Mat& right, int ndisp, int block_size) {
  if (left.empty() || right.empty() || left.type() != CV_8U || right.type() != CV_8U || left.size() != right.size())
    throw std::runtime_error("Stereo input must be equal-size 8-bit grayscale images");
  check_size(left.cols, left.rows);
  if (ndisp <= 0 || ndisp > 2048 || block_size < 3 || block_size > 21 || block_size % 2 == 0)
    throw std::runtime_error("Invalid SGBM parameters");
  const int effective = ((ndisp + 15) / 16) * 16;
  if (effective >= left.cols) throw std::runtime_error("Image width must exceed rounded disparity range");
  if (static_cast<std::uint64_t>(left.cols) * left.rows * effective > 1'000'000'000ULL)
    throw std::runtime_error("Stereo image and disparity range exceed work budget");
  auto stereo = cv::StereoSGBM::create(0, effective, block_size, 8 * block_size * block_size,
                                      32 * block_size * block_size, 1, 63, 10, 100, 32,
                                      cv::StereoSGBM::MODE_SGBM);
  cv::Mat fixed, result;
  stereo->compute(left, right, fixed);
  fixed.convertTo(result, CV_32F, 1.0 / 16.0);
  for (int y = 0; y < result.rows; ++y) for (int x = 0; x < result.cols; ++x)
    if (fixed.at<std::int16_t>(y, x) <= -16) result.at<float>(y, x) = std::numeric_limits<float>::infinity();
  return result;
}

StereoProfile stereo_profile(const std::string& name) {
  if (name == "baseline") return {name, 5, 10, 100, 32, cv::StereoSGBM::MODE_SGBM, false};
  if (name == "quality") return {name, 5, 8, 100, 32, cv::StereoSGBM::MODE_SGBM, true};
  if (name == "fast") return {name, 3, 10, 50, 32, cv::StereoSGBM::MODE_SGBM_3WAY, false};
  if (name == "full") return {name, 5, 10, 100, 32, cv::StereoSGBM::MODE_HH, false};
  throw std::runtime_error("Unknown stereo profile: " + name);
}

ConsistencyStats filter_left_right_consistency(cv::Mat& left_disparity,
                                               const cv::Mat& right_disparity,
                                               float tolerance_px) {
  if (left_disparity.empty() || left_disparity.type() != CV_32F ||
      right_disparity.type() != CV_32F || left_disparity.size() != right_disparity.size() ||
      !std::isfinite(tolerance_px) || tolerance_px < 0)
    throw std::runtime_error("Invalid left-right consistency inputs");
  ConsistencyStats stats;
  const float invalid = std::numeric_limits<float>::infinity();
  double agreement = 0;
  for (int y = 0; y < left_disparity.rows; ++y) for (int x = 0; x < left_disparity.cols; ++x) {
    float& d = left_disparity.at<float>(y, x);
    if (!std::isfinite(d) || d < 0) { d = invalid; continue; }
    ++stats.input_valid;
    const double target = static_cast<double>(x) - d;
    if (target < -0.5 || target >= right_disparity.cols - 0.5) { d = invalid; continue; }
    const int xr = static_cast<int>(std::lround(target));
    const float dr = right_disparity.at<float>(y, xr);
    if (!std::isfinite(dr) || dr > 0 || std::abs(d + dr) > tolerance_px) {
      d = invalid;
      continue;
    }
    ++stats.retained;
    agreement += std::abs(d + dr);
  }
  if (stats.retained) stats.mean_agreement_px = agreement / stats.retained;
  return stats;
}

cv::Mat compute_sgbm_profile(const cv::Mat& left, const cv::Mat& right, int ndisp,
                             const StereoProfile& profile, double* match_ms,
                             double* filter_ms, ConsistencyStats* stats) {
  const StereoProfile expected = stereo_profile(profile.name);
  if (profile.block_size != expected.block_size || profile.uniqueness_ratio != expected.uniqueness_ratio ||
      profile.speckle_window_size != expected.speckle_window_size || profile.speckle_range != expected.speckle_range ||
      profile.mode != expected.mode || profile.left_right_filter != expected.left_right_filter)
    throw std::runtime_error("Stereo profile parameters must match predefined choice");
  if (profile.name == "baseline") {
    const auto start = std::chrono::steady_clock::now();
    cv::Mat result = compute_sgbm(left, right, ndisp);
    if (match_ms) *match_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
    if (filter_ms) *filter_ms = 0;
    if (stats) *stats = {};
    return result;
  }
  if (left.empty() || right.empty() || left.type() != CV_8U || right.type() != CV_8U || left.size() != right.size())
    throw std::runtime_error("Stereo input must be equal-size 8-bit grayscale images");
  check_size(left.cols, left.rows);
  if (ndisp <= 0 || ndisp > 2048) throw std::runtime_error("Invalid SGBM parameters");
  const int effective = ((ndisp + 15) / 16) * 16;
  if (effective >= left.cols || static_cast<std::uint64_t>(left.cols) * left.rows * effective > 1'000'000'000ULL)
    throw std::runtime_error("Stereo image and disparity range exceed work budget");
  if (profile.mode == cv::StereoSGBM::MODE_HH &&
      static_cast<std::uint64_t>(left.cols) * left.rows * effective * 32 > 1024ULL * 1024 * 1024)
    throw std::runtime_error("Full-direction stereo exceeds 1 GiB estimated memory budget");
  const auto start = std::chrono::steady_clock::now();
  auto matcher = [&](int minimum) {
    const int b = profile.block_size;
    return cv::StereoSGBM::create(minimum, effective, b, 8*b*b, 32*b*b, 1, 63,
                                  profile.uniqueness_ratio, profile.speckle_window_size,
                                  profile.speckle_range, profile.mode);
  };
  cv::Mat fixed_left, prediction;
  matcher(0)->compute(left, right, fixed_left);
  fixed_left.convertTo(prediction, CV_32F, 1.0 / 16.0);
  for (int y = 0; y < prediction.rows; ++y) for (int x = 0; x < prediction.cols; ++x)
    if (fixed_left.at<std::int16_t>(y, x) <= -16)
      prediction.at<float>(y, x) = std::numeric_limits<float>::infinity();
  if (match_ms) *match_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
  if (filter_ms) *filter_ms = 0;
  if (stats) *stats = {};
  if (profile.left_right_filter) {
    // Right-view disparity is negative: x_left = x_right - d_right.
    // Its valid search domain is [-(D-1), 0]; min-1 is OpenCV's invalid sentinel.
    cv::Mat fixed_right, right_disparity;
    const int minimum = 1 - effective;
    matcher(minimum)->compute(right, left, fixed_right);
    fixed_right.convertTo(right_disparity, CV_32F, 1.0 / 16.0);
    for (int y = 0; y < right_disparity.rows; ++y) for (int x = 0; x < right_disparity.cols; ++x)
      if (fixed_right.at<std::int16_t>(y, x) <= (minimum - 1) * 16)
        right_disparity.at<float>(y, x) = std::numeric_limits<float>::infinity();
    if (match_ms) *match_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
    const auto filter_start = std::chrono::steady_clock::now();
    const ConsistencyStats measured = filter_left_right_consistency(prediction, right_disparity);
    if (stats) *stats = measured;
    if (filter_ms) *filter_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - filter_start).count();
  }
  return prediction;
}

cv::Mat disparity_to_depth(const cv::Mat& disparity, double fx, double baseline, double doffs) {
  if (disparity.empty() || disparity.type() != CV_32F || !std::isfinite(fx) || fx <= 0 ||
      !std::isfinite(baseline) || baseline <= 0 || !std::isfinite(doffs))
    throw std::runtime_error("Invalid disparity or depth calibration");
  cv::Mat depth(disparity.size(), CV_32F, cv::Scalar(std::numeric_limits<float>::infinity()));
  for (int y = 0; y < disparity.rows; ++y) for (int x = 0; x < disparity.cols; ++x) {
    const double d = disparity.at<float>(y, x);
    const double denom = d + doffs;
    if (std::isfinite(d) && d >= 0 && denom > 0) {
      const double z = fx * baseline / denom;
      if (std::isfinite(z) && z > 0 && z <= std::numeric_limits<float>::max())
        depth.at<float>(y, x) = static_cast<float>(z);
    }
  }
  return depth;
}
} // namespace crispmesh::evaluation
