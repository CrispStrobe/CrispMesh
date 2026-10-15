#include "stereo_eval.hpp"

#include <opencv2/core.hpp>

#include <bit>
#undef NDEBUG
#include <cassert>
#include <cmath>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <limits>
#include <random>
#include <stdexcept>
#include <string>
#include <thread>
#include <atomic>

using namespace crispmesh::evaluation;
namespace fs = std::filesystem;

namespace {
void expect_error(auto&& action) {
  bool failed = false;
  try { action(); } catch (const std::exception&) { failed = true; }
  assert(failed);
}

void write_big_endian(std::ofstream& out, float f) {
  const std::uint32_t n = std::bit_cast<std::uint32_t>(f);
  const unsigned char bytes[] = {static_cast<unsigned char>(n >> 24), static_cast<unsigned char>(n >> 16),
                                 static_cast<unsigned char>(n >> 8), static_cast<unsigned char>(n)};
  out.write(reinterpret_cast<const char*>(bytes), 4);
}
void write_little_endian(std::ofstream& out, float f) {
  const std::uint32_t n = std::bit_cast<std::uint32_t>(f);
  const unsigned char bytes[] = {static_cast<unsigned char>(n), static_cast<unsigned char>(n >> 8),
                                 static_cast<unsigned char>(n >> 16), static_cast<unsigned char>(n >> 24)};
  out.write(reinterpret_cast<const char*>(bytes), 4);
}
} // namespace

int main() {
  cv::Mat truth(1, 5, CV_32F);
  cv::Mat pred(1, 5, CV_32F);
  truth.at<float>(0, 0) = 0; // Zero disparity is valid.
  truth.at<float>(0, 1) = 3;
  truth.at<float>(0, 2) = 4;
  truth.at<float>(0, 3) = std::numeric_limits<float>::infinity();
  truth.at<float>(0, 4) = 9;
  pred.at<float>(0, 0) = 0;
  pred.at<float>(0, 1) = 6;
  pred.at<float>(0, 2) = std::numeric_limits<float>::infinity();
  pred.at<float>(0, 3) = 1;
  pred.at<float>(0, 4) = 9;
  const Metrics all = evaluate_disparity(pred, truth);
  assert(all.roi_pixels == 5 && all.gt_valid == 4 && all.matched == 3 && all.bad2_matched == 1);
  assert(std::abs(all.coverage - 0.75) < 1e-9);
  assert(std::abs(all.bad2_all_valid - 0.5) < 1e-9);
  assert(std::abs(all.bad2_matched_rate - 1.0/3) < 1e-9);
  cv::Mat mask(1, 5, CV_8U, cv::Scalar(0));
  mask.at<std::uint8_t>(0, 0) = mask.at<std::uint8_t>(0, 2) = 255;
  const Metrics region = evaluate_disparity(pred, truth, mask);
  assert(region.roi_pixels == 2 && region.gt_valid == 2 && region.matched == 1);
  assert(region.coverage == 0.5 && region.bad2_all_valid == 0.5);
  expect_error([&] { evaluate_disparity(pred, truth, cv::Mat(2, 5, CV_8U)); });

  cv::Mat left(48, 128, CV_8U), right(48, 128, CV_8U);
  for (int y = 0; y < left.rows; ++y) for (int x = 0; x < left.cols; ++x)
    left.at<std::uint8_t>(y, x) = static_cast<std::uint8_t>((x * 57 + y * 113 + x * y * 19) & 255);
  for (int y = 0; y < right.rows; ++y) for (int x = 0; x < right.cols; ++x)
    right.at<std::uint8_t>(y, x) = left.at<std::uint8_t>(y, std::min(x + 8, left.cols - 1));
  const cv::Mat disparity = compute_sgbm(left, right, 16);
  int good = 0;
  for (int y = 8; y < 40; ++y) for (int x = 40; x < 110; ++x)
    if (std::isfinite(disparity.at<float>(y, x)) &&
        std::abs(disparity.at<float>(y, x) - 8) < 1) ++good;
  assert(good > 1800);
  expect_error([&] { compute_sgbm(left, right, 4096); });
  const auto baseline_profile = stereo_profile("baseline");
  assert(baseline_profile.block_size == 5 && baseline_profile.uniqueness_ratio == 10 &&
         baseline_profile.speckle_window_size == 100 && !baseline_profile.left_right_filter);
  const cv::Mat explicit_baseline = compute_sgbm_profile(left, right, 16, baseline_profile);
  for (int y = 0; y < disparity.rows; ++y) for (int x = 0; x < disparity.cols; ++x)
    assert(std::bit_cast<std::uint32_t>(disparity.at<float>(y, x)) ==
           std::bit_cast<std::uint32_t>(explicit_baseline.at<float>(y, x)));
  assert(stereo_profile("quality").left_right_filter && !stereo_profile("fast").left_right_filter);
  expect_error([&] { stereo_profile("unknown"); });
  auto forged = baseline_profile; forged.block_size = 3;
  expect_error([&] { compute_sgbm_profile(left, right, 16, forged); });
  cv::Mat consistency_left(1, 7, CV_32F, cv::Scalar(std::numeric_limits<float>::infinity()));
  cv::Mat consistency_right(1, 7, CV_32F, cv::Scalar(std::numeric_limits<float>::infinity()));
  consistency_left.at<float>(0, 0) = 0; consistency_right.at<float>(0, 0) = 0;
  consistency_left.at<float>(0, 3) = 2; consistency_right.at<float>(0, 1) = -2;
  consistency_left.at<float>(0, 5) = 1; consistency_right.at<float>(0, 4) = -3;
  consistency_left.at<float>(0, 6) = 6; // Right pixel zero disagrees: occluded.
  const auto consistent = filter_left_right_consistency(consistency_left, consistency_right);
  assert(consistent.input_valid == 4 && consistent.retained == 2);
  assert(consistency_left.at<float>(0, 0) == 0 && consistency_left.at<float>(0, 3) == 2);
  assert(std::isinf(consistency_left.at<float>(0, 5)) && std::isinf(consistency_left.at<float>(0, 6)));
  consistency_left.at<float>(0, 2) = std::numeric_limits<float>::max();
  consistency_left.at<float>(0, 1) = -1;
  const auto bounded = filter_left_right_consistency(consistency_left, consistency_right);
  assert(bounded.input_valid == 3 && bounded.retained == 2);
  assert(std::isinf(consistency_left.at<float>(0, 2)) && std::isinf(consistency_left.at<float>(0, 1)));
  expect_error([&] { filter_left_right_consistency(consistency_left, consistency_right, -1); });
  cv::setNumThreads(1);
  const auto caller_thread = std::this_thread::get_id();
  std::atomic<int> other_threads{0};
  cv::parallel_for_(cv::Range(0, 128), [&](const cv::Range&) {
    if (std::this_thread::get_id() != caller_thread) ++other_threads;
  });
  assert(other_threads == 0);
  cv::setNumThreads(-1);

  cv::Mat tiny(1, 3, CV_32F);
  tiny.at<float>(0, 0) = 2;
  tiny.at<float>(0, 1) = 0;
  tiny.at<float>(0, 2) = std::numeric_limits<float>::infinity();
  const cv::Mat depth = disparity_to_depth(tiny, 10, 5, 3);
  assert(depth.at<float>(0, 0) == 10 && std::abs(depth.at<float>(0, 1) - 50.0f/3) < 1e-5);
  assert(std::isinf(depth.at<float>(0, 2)));
  const cv::Mat invalid_depth = disparity_to_depth(tiny, 10, 5, -1);
  assert(std::isinf(invalid_depth.at<float>(0, 1)));

  fs::path temp;
  std::random_device random;
  bool created = false;
  for (int attempt = 0; attempt < 10; ++attempt) {
    temp = fs::temp_directory_path() / ("crispmesh-stereo-eval-" + std::to_string(random()) + "-" + std::to_string(attempt));
    if (fs::create_directory(temp)) { created = true; break; }
  }
  assert(created);
  cv::Mat image(2, 2, CV_32F);
  image.at<float>(0, 0) = 2;
  image.at<float>(0, 1) = std::numeric_limits<float>::infinity();
  image.at<float>(1, 0) = 4;
  image.at<float>(1, 1) = std::numeric_limits<float>::quiet_NaN();
  write_pfm(temp / "roundtrip.pfm", image);
  const cv::Mat copy = read_pfm(temp / "roundtrip.pfm");
  assert(copy.at<float>(0, 0) == 2 && copy.at<float>(1, 0) == 4);
  assert(std::isinf(copy.at<float>(0, 1)) && std::isnan(copy.at<float>(1, 1)));
  {
    std::ofstream out(temp / "big.pfm", std::ios::binary);
    out << "Pf\n2 1\n0.003922\n";
    write_big_endian(out, 1.5f);
    write_big_endian(out, std::numeric_limits<float>::infinity());
  }
  const cv::Mat big = read_pfm(temp / "big.pfm");
  assert(big.at<float>(0, 0) == 1.5 && std::isinf(big.at<float>(0, 1)));
  {
    std::ofstream out(temp / "little-scale.pfm", std::ios::binary);
    out << "Pf\n1 1\n-0.003922\n";
    write_little_endian(out, 123.5f);
  }
  assert(read_pfm(temp / "little-scale.pfm").at<float>(0, 0) == 123.5f);
  {
    std::ofstream out(temp / "huge.pfm", std::ios::binary);
    out << "Pf\n999999999999999999999 2\n-1.0\n";
  }
  expect_error([&] { read_pfm(temp / "huge.pfm"); });
  {
    std::ofstream out(temp / "bad-calib.txt");
    out << "cam0=[1 0 0; 0 1 0; 0 0 1]\nbaseline=1\ndoffs=0\nndisp=1e100\nwidth=2\nheight=2\n";
  }
  expect_error([&] { read_middlebury_calibration(temp / "bad-calib.txt"); });
  fs::remove(temp / "roundtrip.pfm");
  fs::remove(temp / "big.pfm");
  fs::remove(temp / "little-scale.pfm");
  fs::remove(temp / "huge.pfm");
  fs::remove(temp / "bad-calib.txt");
  fs::remove(temp);
}
