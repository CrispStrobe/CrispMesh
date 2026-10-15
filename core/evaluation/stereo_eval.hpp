#pragma once

#include <opencv2/core.hpp>

#include <cstddef>
#include <filesystem>
#include <string>

namespace crispmesh::evaluation {

struct Calibration {
  double fx = 0;
  double baseline = 0;
  double doffs = 0;
  int ndisp = 0;
  int width = 0;
  int height = 0;
};

struct Metrics {
  std::size_t roi_pixels = 0;
  std::size_t gt_valid = 0;
  std::size_t matched = 0;
  std::size_t bad2_matched = 0;
  double mean_abs_error_matched = 0;
  double rms_error_matched = 0;
  double coverage = 0;
  double bad2_all_valid = 0;
  double bad2_matched_rate = 0;
};

struct StereoProfile {
  std::string name;
  int block_size;
  int uniqueness_ratio;
  int speckle_window_size;
  int speckle_range;
  int mode;
  bool left_right_filter;
};

struct ConsistencyStats {
  std::size_t input_valid = 0;
  std::size_t retained = 0;
  double mean_agreement_px = 0;
};

StereoProfile stereo_profile(const std::string& name);
cv::Mat compute_sgbm_profile(const cv::Mat& left, const cv::Mat& right, int ndisp,
                             const StereoProfile& profile, double* match_ms = nullptr,
                             double* filter_ms = nullptr, ConsistencyStats* stats = nullptr);
ConsistencyStats filter_left_right_consistency(cv::Mat& left_disparity,
                                               const cv::Mat& right_disparity,
                                               float tolerance_px = 1.0f);

Calibration read_middlebury_calibration(const std::filesystem::path& path);
cv::Mat read_pfm(const std::filesystem::path& path);
void write_pfm(const std::filesystem::path& path, const cv::Mat& image);
Metrics evaluate_disparity(const cv::Mat& prediction, const cv::Mat& truth,
                           const cv::Mat& mask = {});
cv::Mat compute_sgbm(const cv::Mat& left, const cv::Mat& right, int ndisp,
                     int block_size = 5);
cv::Mat disparity_to_depth(const cv::Mat& disparity, double fx, double baseline, double doffs);

} // namespace crispmesh::evaluation
