#include "stereo_eval.hpp"

#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/core/version.hpp>
#include <opencv2/core.hpp>

#include <algorithm>
#include <initializer_list>
#include <chrono>
#include <cmath>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <map>
#include <stdexcept>
#include <string>
#include <cstdint>
#if defined(__APPLE__) || defined(__linux__)
#include <sys/resource.h>
#endif

namespace fs = std::filesystem;
using namespace crispmesh::evaluation;

namespace {
void usage() {
  std::cerr << "Usage: crispmesh_stereo_eval --gt disparity.pfm --output-dir DIR "
               "[--left im0.png --right im1.png (--calib calib.txt | --fx N --baseline N --doffs N --ndisp N) "
               "| --prediction disparity.pfm] [--mask mask.png] [--downsample 1|2|4] [--prepare-only yes] [--profile baseline|quality|fast|full --threads N]\n";
}

cv::Mat bounded_gray_image(const fs::path& path) {
  std::ifstream in(path, std::ios::binary);
  unsigned char header[24] = {};
  in.read(reinterpret_cast<char*>(header), sizeof header);
  if (header[0] == 'P' && header[1] == '5') {
    in.clear(); in.seekg(0);
    std::string magic;
    int width = 0, height = 0, max_value = 0;
    in >> magic >> width >> height >> max_value;
    if (!in || magic != "P5" || width <= 0 || height <= 0 ||
        static_cast<std::uint64_t>(width) * height > 25'000'000ULL || max_value != 255 || in.get() != '\n')
      throw std::runtime_error("Invalid or oversized P5 PGM: " + path.string());
    cv::Mat image(height, width, CV_8U);
    in.read(reinterpret_cast<char*>(image.data), static_cast<std::streamsize>(image.total()));
    if (!in) throw std::runtime_error("Truncated PGM: " + path.string());
    return image;
  }
  const unsigned char signature[] = {137,80,78,71,13,10,26,10};
  if (!in || !std::equal(std::begin(signature), std::end(signature), header))
    throw std::runtime_error("Expected PNG input: " + path.string());
  auto be32 = [](const unsigned char* b) { return (std::uint64_t(b[0]) << 24) | (std::uint64_t(b[1]) << 16) | (std::uint64_t(b[2]) << 8) | b[3]; };
  const auto width = be32(header + 16), height = be32(header + 20);
  if (width == 0 || height == 0 || width * height > 25'000'000ULL)
    throw std::runtime_error("PNG exceeds evaluation image bound");
  cv::Mat image = cv::imread(path.string(), cv::IMREAD_GRAYSCALE);
  if (image.empty() || image.cols != static_cast<int>(width) || image.rows != static_cast<int>(height))
    throw std::runtime_error("Cannot decode PNG: " + path.string());
  return image;
}

void write_pgm(const fs::path& path, const cv::Mat& image) {
  if (image.empty() || image.type() != CV_8U || !image.isContinuous())
    throw std::runtime_error("PGM requires continuous 8-bit grayscale image");
  std::ofstream out(path, std::ios::binary);
  if (!out) throw std::runtime_error("Cannot write PGM: " + path.string());
  out << "P5\n" << image.cols << ' ' << image.rows << "\n255\n";
  out.write(reinterpret_cast<const char*>(image.data), static_cast<std::streamsize>(image.total()));
  if (!out) throw std::runtime_error("Failed writing PGM");
}

void ensure_new_outputs(const fs::path& output, std::initializer_list<const char*> names) {
  if (fs::is_symlink(fs::symlink_status(output))) throw std::runtime_error("Output directory may not be a symlink");
  for (const char* name : names)
    if (fs::exists(fs::symlink_status(output / name)))
      throw std::runtime_error(std::string("Output already exists: ") + (output / name).string());
  fs::create_directories(output);
}

double numeric(const std::string& s, const char* key) {
  std::size_t consumed = 0;
  double x = std::stod(s, &consumed);
  if (consumed != s.size() || !std::isfinite(x)) throw std::runtime_error(std::string("Invalid ") + key);
  return x;
}

std::string json_path(const fs::path& p) {
  std::string out = "\"";
  for (unsigned char c : p.string()) {
    if (c == '"' || c == '\\') out += '\\';
    if (c < 0x20) throw std::runtime_error("Control character in path");
    out += static_cast<char>(c);
  }
  return out + '"';
}
} // namespace

int main(int argc, char** argv) {
  try {
    std::map<std::string, std::string> args;
    for (int i = 1; i < argc; i += 2) {
      if (i + 1 >= argc || std::string(argv[i]).rfind("--", 0) != 0) {
        usage(); return 2;
      }
      const std::string key = argv[i] + 2;
      if (!args.emplace(key, argv[i + 1]).second) throw std::runtime_error("Duplicate option: " + key);
    }
    for (const auto& [key, value] : args)
      if (key != "left" && key != "right" && key != "gt" && key != "output-dir" &&
          key != "calib" && key != "fx" && key != "baseline" && key != "doffs" &&
          key != "ndisp" && key != "mask" && key != "prediction" && key != "downsample" &&
          key != "prepare-only" && key != "profile" && key != "threads")
        throw std::runtime_error("Unknown option: " + key);
    if (!args.count("gt") || !args.count("output-dir")) { usage(); return 2; }
    const bool oracle = args.count("prediction");
    const bool prepare = args.count("prepare-only");
    const bool profiled = args.count("profile");
    if (profiled && (oracle || prepare)) throw std::runtime_error("Profile requires a stereo scoring run");
    if (args.count("threads") && !profiled) throw std::runtime_error("--threads requires --profile");
    if (args.count("threads")) {
      const double count = numeric(args.at("threads"), "threads");
      if (count < 1 || count > std::min(64, cv::getNumberOfCPUs()) || std::floor(count) != count)
        throw std::runtime_error("Threads must be an integer from 1 through bounded CPU count");
      cv::setNumThreads(static_cast<int>(count));
    }
    const StereoProfile profile = stereo_profile(profiled ? args.at("profile") : "baseline");
    if (prepare && (args.at("prepare-only") != "yes" || oracle))
      throw std::runtime_error("--prepare-only accepts yes and requires stereo images");
    const double downsample_number = args.count("downsample") ? numeric(args.at("downsample"), "downsample") : 1;
    if ((downsample_number != 1 && downsample_number != 2 && downsample_number != 4) || (oracle && downsample_number != 1))
      throw std::runtime_error("Downsample must be 1, 2 or 4 and requires stereo mode");
    const int downsample = static_cast<int>(downsample_number);
    if (oracle ? (args.count("left") || args.count("right")) : (!args.count("left") || !args.count("right"))) {
      usage(); return 2;
    }
    const bool has_calib = args.count("calib");
    const bool full_values = args.count("fx") || args.count("baseline") || args.count("doffs");
    const bool has_ndisp = args.count("ndisp");
    const bool pixel_only = has_ndisp && !full_values && !has_calib;
    const bool simple = full_values || pixel_only;
    if (has_calib && (full_values || has_ndisp)) throw std::runtime_error("Choose calibration file or explicit calibration values");
    if (!oracle && !has_calib && !has_ndisp)
      throw std::runtime_error("Stereo run requires calibration or pixel disparity range");

    Calibration c;
    if (has_calib) c = read_middlebury_calibration(args.at("calib"));
    if (simple) {
      if (full_values && !(args.count("fx") && args.count("baseline") && args.count("doffs") && args.count("ndisp")))
        throw std::runtime_error("Explicit calibration requires fx, baseline, doffs and ndisp");
      if (full_values) {
        c.fx = numeric(args.at("fx"), "fx");
        c.baseline = numeric(args.at("baseline"), "baseline");
        c.doffs = numeric(args.at("doffs"), "doffs");
      }
      const double range = numeric(args.at("ndisp"), "ndisp");
      if (range < 1 || range > 2048 || std::floor(range) != range) throw std::runtime_error("Invalid ndisp");
      c.ndisp = static_cast<int>(range);
    }
    if ((has_calib || full_values) && (c.fx <= 0 || c.baseline <= 0 || c.ndisp < 1 || c.ndisp > 2048))
      throw std::runtime_error("Invalid calibration");

    const auto decode_start = std::chrono::steady_clock::now();
    cv::Mat truth = read_pfm(args.at("gt"));
    const int source_width = truth.cols, source_height = truth.rows;
    cv::Mat mask;
    if (args.count("mask")) {
      mask = bounded_gray_image(args.at("mask"));
      if (mask.empty() || mask.size() != truth.size()) throw std::runtime_error("Mask must be an equal-size PNG");
    }
    cv::Mat prediction;
    double elapsed_ms = 0;
    double decode_ms = 0, preprocess_ms = 0, match_ms = 0, filter_ms = 0, score_ms = 0;
    ConsistencyStats consistency;
    if (oracle) {
      prediction = read_pfm(args.at("prediction"));
    } else {
      cv::Mat left = bounded_gray_image(args.at("left"));
      cv::Mat right = bounded_gray_image(args.at("right"));
      if (left.empty() || right.empty() || left.size() != truth.size() || right.size() != truth.size())
        throw std::runtime_error("Stereo PNG dimensions must match ground truth");
      if (has_calib && (c.width != left.cols || c.height != left.rows))
        throw std::runtime_error("Calibration dimensions do not match images");
      const auto preprocess_start = std::chrono::steady_clock::now();
      decode_ms = std::chrono::duration<double, std::milli>(preprocess_start - decode_start).count();
      if (downsample > 1) {
        if (left.cols % downsample || left.rows % downsample)
          throw std::runtime_error("Downsample requires source dimensions divisible by factor");
        const cv::Size size(left.cols / downsample, left.rows / downsample);
        cv::resize(left, left, size, 0, 0, cv::INTER_AREA);
        cv::resize(right, right, size, 0, 0, cv::INTER_AREA);
        cv::resize(truth, truth, size, 0, 0, cv::INTER_NEAREST);
        truth /= static_cast<float>(downsample);
        if (!mask.empty()) cv::resize(mask, mask, size, 0, 0, cv::INTER_NEAREST);
        c.fx /= downsample;
        c.doffs /= downsample;
        c.ndisp = static_cast<int>(std::ceil(static_cast<double>(c.ndisp) / downsample));
      }
      if (prepare) {
        const fs::path output = args.at("output-dir");
        if (mask.empty()) ensure_new_outputs(output, {"left.pgm", "right.pgm", "truth.pfm", "inputs.json"});
        else ensure_new_outputs(output, {"left.pgm", "right.pgm", "truth.pfm", "mask.pgm", "inputs.json"});
        write_pgm(output / "left.pgm", left);
        write_pgm(output / "right.pgm", right);
        write_pfm(output / "truth.pfm", truth);
        if (!mask.empty()) write_pgm(output / "mask.pgm", mask);
        std::ofstream manifest(output / "inputs.json");
        if (!manifest) throw std::runtime_error("Cannot write inputs.json");
        manifest << std::setprecision(12)
                 << "{\n  \"schema_version\": 1,\n  \"left\": \"left.pgm\",\n"
                 << "  \"right\": \"right.pgm\",\n  \"truth\": \"truth.pfm\",\n"
                 << "  \"mask\": " << (mask.empty() ? "null" : "\"mask.pgm\"") << ",\n"
                 << "  \"source_left\": " << json_path(args.at("left")) << ",\n"
                 << "  \"source_right\": " << json_path(args.at("right")) << ",\n"
                 << "  \"source_gt\": " << json_path(args.at("gt")) << ",\n"
                 << "  \"source_calib\": " << (has_calib ? json_path(args.at("calib")) : "null") << ",\n"
                 << "  \"source_mask\": " << (args.count("mask") ? json_path(args.at("mask")) : "null") << ",\n"
                 << "  \"width\": " << left.cols << ", \"height\": " << left.rows << ",\n"
                 << "  \"source_width\": " << source_width << ", \"source_height\": " << source_height << ",\n"
                 << "  \"downsample_factor\": " << downsample << ",\n"
                 << "  \"ndisp\": " << c.ndisp << ",\n"
                 << "  \"sampling\": \"OpenCV IMREAD_GRAYSCALE; INTER_AREA grayscale images; INTER_NEAREST GT and evaluation mask; disparity/fx/doffs/ndisp divided by factor (ndisp rounded up)\",\n"
                 << "  \"calibration\": ";
        if (has_calib || full_values) manifest << "{\"fx\":" << c.fx << ",\"baseline\":" << c.baseline
                                               << ",\"doffs\":" << c.doffs << ",\"ndisp\":" << c.ndisp << '}';
        else manifest << "null";
        manifest << "\n}\n";
        if (!manifest) throw std::runtime_error("Failed writing inputs.json");
        std::cout << (output / "inputs.json").string() << '\n';
        return 0;
      }
      const auto start = std::chrono::steady_clock::now();
      preprocess_ms = std::chrono::duration<double, std::milli>(start - preprocess_start).count();
      if (profiled)
        prediction = compute_sgbm_profile(left, right, c.ndisp, profile, &match_ms, &filter_ms, &consistency);
      else prediction = compute_sgbm(left, right, c.ndisp);
      elapsed_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - start).count();
    }
    const auto score_start = std::chrono::steady_clock::now();
    const Metrics m = evaluate_disparity(prediction, truth, mask);
    score_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - score_start).count();
    if (m.gt_valid == 0) throw std::runtime_error("No valid ground-truth disparities in evaluation region");
    const fs::path output = args.at("output-dir");
    ensure_new_outputs(output, {"metrics.json", "disparity.pfm", "depth.pfm"});
    write_pfm(output / "disparity.pfm", prediction);
    if (has_calib || full_values)
      write_pfm(output / "depth.pfm", disparity_to_depth(prediction, c.fx, c.baseline, c.doffs));
    std::ofstream out(output / "metrics.json");
    if (!out) throw std::runtime_error("Cannot write metrics.json");
    out << std::setprecision(12)
        << "{\n  \"schema_version\": 1,\n  \"mode\": \"" << (oracle ? "prediction" : "opencv_sgbm") << "\",\n"
        << "  \"ground_truth\": " << json_path(args.at("gt")) << ",\n"
        << "  \"prediction_source\": " << (oracle ? json_path(args.at("prediction")) : "null") << ",\n"
        << "  \"mask\": " << (args.count("mask") ? json_path(args.at("mask")) : "null") << ",\n"
        << "  \"mask_role\": \"evaluation_only\",\n"
        << "  \"left\": " << (oracle ? "null" : json_path(args.at("left"))) << ", \"right\": " << (oracle ? "null" : json_path(args.at("right"))) << ",\n"
        << "  \"opencv_version\": \"" << CV_VERSION << "\",\n"
        << "  \"source_width\": " << source_width << ", \"source_height\": " << source_height << ",\n"
        << "  \"downsample_factor\": " << downsample << ",\n"
        << "  \"sampling\": \"INTER_AREA PNG images; INTER_NEAREST GT and evaluation mask; disparity/fx/doffs/ndisp divided by factor (ndisp rounded up)\",\n"
        << "  \"error_unit\": \"pixels at evaluated resolution\",\n"
        << "  \"width\": " << truth.cols << ", \"height\": " << truth.rows << ",\n"
        << "  \"roi_pixels\": " << m.roi_pixels << ", \"gt_valid\": " << m.gt_valid
        << ", \"matched\": " << m.matched << ", \"bad2_matched_count\": " << m.bad2_matched << ",\n"
        << "  \"coverage\": " << m.coverage << ", \"bad2_all_valid\": " << m.bad2_all_valid
        << ", \"bad2_matched\": ";
    if (m.matched) out << m.bad2_matched_rate; else out << "null";
    out << ",\n"
        << "  \"mae_matched_px\": ";
    if (m.matched) out << m.mean_abs_error_matched; else out << "null";
    out << ", \"rmse_matched_px\": ";
    if (m.matched) out << m.rms_error_matched; else out << "null";
    out << ",\n"
        << "  \"elapsed_stereo_ms\": " << elapsed_ms << ",\n"
        << "  \"calibration\": ";
    if (has_calib || full_values) out << "{\"fx\":" << c.fx << ",\"baseline\":" << c.baseline
                                 << ",\"doffs\":" << c.doffs << ",\"ndisp\":" << c.ndisp << '}';
    else out << "null";
    out << ",\n  \"depth_formula\": " << ((has_calib || full_values) ? "\"Z=fx*baseline/(d+doffs)\"" : "null") << ",\n"
        << "  \"sgbm\": ";
    if (oracle) out << "null";
    else out << "{\"min_disparity\":0,\"block_size\":" << profile.block_size
             << ",\"P1\":" << 8 * profile.block_size * profile.block_size
             << ",\"P2\":" << 32 * profile.block_size * profile.block_size
             << ",\"disp12_max_diff\":1,\"pre_filter_cap\":63,\"uniqueness_ratio\":" << profile.uniqueness_ratio
             << ",\"speckle_window_size\":" << profile.speckle_window_size
             << ",\"speckle_range\":" << profile.speckle_range
             << ",\"mode\":\"" << (profile.name == "fast" ? "SGBM_3WAY" : profile.name == "full" ? "HH" : "SGBM")
             << "\",\"num_disparities\":" << ((c.ndisp + 15) / 16) * 16 << '}';
    if (profiled) {
      const std::uint64_t pixels = static_cast<std::uint64_t>(truth.cols) * truth.rows;
      const std::uint64_t count = ((c.ndisp + 15) / 16) * 16;
      // Conservative algorithmic allocation estimate, not process RSS.
      const std::uint64_t estimate = pixels * (16 + (profile.name == "full" ? 32 : 16) * count * (profile.left_right_filter ? 2 : 1));
      out << ",\n  \"profile\": \"" << profile.name << "\",\n"
          << "  \"left_right_consistency\": {\"enabled\":" << (profile.left_right_filter ? "true" : "false")
          << ",\"threshold_px\":1,\"input_valid\":" << consistency.input_valid
          << ",\"retained\":" << consistency.retained
          << ",\"retained_fraction\":" << (consistency.input_valid ? static_cast<double>(consistency.retained) / consistency.input_valid : 0)
          << ",\"mean_agreement_px\":" << consistency.mean_agreement_px << "},\n"
          << "  \"stage_ms\": {\"decode\":" << decode_ms << ",\"preprocess\":" << preprocess_ms
          << ",\"match\":" << match_ms << ",\"filter\":" << filter_ms << ",\"score\":" << score_ms << "},\n"
          << "  \"estimated_working_bytes\": " << estimate;
      out << ",\n  \"opencv_requested_threads\": " << (args.count("threads") ? args.at("threads") : "null")
          << ",\n  \"opencv_reported_pool_limit\": " << cv::getNumThreads()
          << ",\n  \"opencv_parallel_for_serial\": " << (args.count("threads") && args.at("threads") == "1" ? "true" : "false");
      out << ",\n  \"process_peak_rss_bytes\": ";
#if defined(__APPLE__) || defined(__linux__)
      struct rusage usage_info {};
      if (getrusage(RUSAGE_SELF, &usage_info) == 0) {
#if defined(__APPLE__)
        out << static_cast<std::uint64_t>(usage_info.ru_maxrss);
#else
        out << static_cast<std::uint64_t>(usage_info.ru_maxrss) * 1024;
#endif
      } else out << "null";
#else
      out << "null";
#endif
    }
    out
        << "\n}\n";
    if (!out) throw std::runtime_error("Failed writing metrics.json");
    std::cout << (output / "metrics.json").string() << '\n';
    return 0;
  } catch (const std::exception& e) {
    std::cerr << "stereo evaluation: " << e.what() << '\n';
    return 1;
  }
}
