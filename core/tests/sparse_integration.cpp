#ifdef NDEBUG
#undef NDEBUG
#endif
#include "crispmesh/core.h"

#include <opencv2/calib3d.hpp>
#include <opencv2/core.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/objdetect/aruco_detector.hpp>

#include <array>
#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

namespace fs = std::filesystem;

namespace {
constexpr int width = 1600, height = 1200;
const cv::Mat K = (cv::Mat_<double>(3, 3) << 1500, 0, width / 2., 0, 1500, height / 2., 0, 0, 1);

struct Plane {
  double x0, y0, x1, y1, z;
};
constexpr std::array<Plane, 2> panels{{{75, 75, 145, 225, -45}, {155, 75, 230, 225, -85}}};
constexpr std::array<std::array<double, 2>, 4> marker_origins{{{{0, 0}}, {{260, 0}}, {{0, 260}}, {{260, 260}}}};

void write_text(const fs::path &path, const std::string &content) {
  std::ofstream file(path, std::ios::binary);
  file << content;
  assert(file.good());
}
std::string read_text(const fs::path &path) {
  std::ifstream file(path, std::ios::binary);
  return {std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>()};
}
std::vector<cv::Point3f> corners(const Plane &p) {
  return {{float(p.x0), float(p.y0), float(p.z)}, {float(p.x1), float(p.y0), float(p.z)},
          {float(p.x1), float(p.y1), float(p.z)}, {float(p.x0), float(p.y1), float(p.z)}};
}
std::vector<cv::Point2f> projection(const Plane &p, const cv::Mat &rvec, const cv::Mat &tvec) {
  std::vector<cv::Point2f> output;
  cv::projectPoints(corners(p), rvec, tvec, K, cv::Mat(), output);
  return output;
}
void composite(cv::Mat &image, const cv::Mat &texture, const std::vector<cv::Point2f> &destination,
               cv::Mat *silhouette = nullptr, int interpolation = cv::INTER_LINEAR) {
  std::vector<cv::Point2f> source{{0, 0}, {float(texture.cols - 1), 0},
                                  {float(texture.cols - 1), float(texture.rows - 1)},
                                  {0, float(texture.rows - 1)}};
  const cv::Mat H = cv::getPerspectiveTransform(source, destination);
  cv::Mat warped(image.size(), image.type(), cv::Scalar(0));
  cv::warpPerspective(texture, warped, H, image.size(), interpolation, cv::BORDER_CONSTANT);
  std::vector<cv::Point> polygon;
  for (const auto &p : destination) polygon.emplace_back(cvRound(p.x), cvRound(p.y));
  cv::Mat coverage(image.size(), CV_8UC1, cv::Scalar(0));
  cv::fillConvexPoly(coverage, polygon, cv::Scalar(255), cv::LINE_AA);
  warped.copyTo(image, coverage);
  if (silhouette) cv::max(*silhouette, coverage, *silhouette);
}
cv::Mat texture(int rows, int cols, uint64_t seed) {
  cv::Mat result(rows, cols, CV_8UC1, cv::Scalar(150));
  cv::RNG rng(seed);
  for (int y = 0; y < rows; y += 6) {
    for (int x = 0; x < cols; x += 6) {
      int value = rng.uniform(45, 220);
      cv::rectangle(result, {x, y}, {std::min(x + 5, cols - 1), std::min(y + 5, rows - 1)},
                    cv::Scalar(value), cv::FILLED);
    }
  }
  for (int n = 0; n < rows * cols / 300; ++n) {
    cv::circle(result, {rng.uniform(0, cols), rng.uniform(0, rows)}, rng.uniform(2, 6),
               cv::Scalar(rng.uniform(0, 2) ? 235 : 20), cv::FILLED, cv::LINE_AA);
  }
  return result;
}
std::string project_json(const std::vector<std::string> &paths, const std::vector<std::string> &masks,
                         const std::string &id = "synthetic-sparse-r02") {
  assert(paths.size() == masks.size());
  std::ostringstream out;
  out << "{\"schemaVersion\":1,\"id\":\"" << id
      << "\",\"name\":\"Synthetic metric sparse fixture\",\"units\":\"mm\",\"images\":[";
  for (size_t i = 0; i < paths.size(); ++i) {
    if (i) out << ',';
    out << "{\"id\":\"view-" << i + 1 << "\",\"path\":\"" << paths[i]
        << "\",\"maskPath\":\"" << masks[i] << "\"}";
  }
  out << "],\"calibration\":{\"width\":" << width << ",\"height\":" << height
      << ",\"fx\":1500,\"fy\":1500,\"cx\":800,\"cy\":600,"
         "\"distortionModel\":\"opencv-radtan\",\"distortion\":[0,0,0,0]},"
         "\"board\":{\"type\":\"aruco\",\"dictionary\":\"DICT_4X4_50\","
         "\"markerSizeMm\":40,\"rotatesWithObject\":true,\"markers\":[";
  for (size_t i = 0; i < marker_origins.size(); ++i) {
    if (i) out << ',';
    const auto [x, y] = marker_origins[i];
    out << "{\"id\":" << i + 1 << ",\"cornersMm\":[[" << x << ',' << y << ",0],["
        << x + 40 << ',' << y << ",0],[" << x + 40 << ',' << y + 40 << ",0],["
        << x << ',' << y + 40 << ",0]]}";
  }
  out << "]},\"stages\":{}}";
  return out.str();
}
std::string api(const fs::path &path, crispmesh_status expected) {
  char *output = nullptr;
  size_t size = 0;
  auto actual = crispmesh_reconstruct_sparse_alloc(path.string().c_str(), &output, &size);
  std::string report = output ? std::string(output) : std::string();
  crispmesh_free(output);
  if (actual != expected) {
    std::cerr << "API status " << actual << " expected " << expected << " for " << path
              << " report: " << report.substr(0, 500) << '\n';
    std::abort();
  }
  assert(size == report.size() + 1);
  return report;
}
cv::FileStorage parse(const std::string &json) {
  cv::FileStorage doc(json, cv::FileStorage::READ | cv::FileStorage::MEMORY | cv::FileStorage::FORMAT_JSON);
  assert(doc.isOpened());
  return doc;
}
double number(const cv::FileNode &n) { return static_cast<double>(n); }
std::array<double, 3> xyz(const cv::FileNode &n) {
  assert(n.isSeq() && n.size() == 3);
  return {number(n[0]), number(n[1]), number(n[2])};
}
bool near_panel(const std::array<double, 3> &p) {
  for (const auto &panel : panels) {
    if (p[0] >= panel.x0 - 12 && p[0] <= panel.x1 + 12 &&
        p[1] >= panel.y0 - 12 && p[1] <= panel.y1 + 12 && std::abs(p[2] - panel.z) < 13)
      return true;
  }
  return false;
}
void expect_failure(const fs::path &root, const fs::path &cli, const std::string &stem,
                    const std::string &project, const std::string &expected_code) {
  const auto project_path = root / (stem + ".json");
  write_text(project_path, project);
  const auto report = api(project_path, CRISPMESH_INVALID_PROJECT);
  auto api_doc = parse(report);
  assert(static_cast<int>(api_doc["ok"]) == 0);
  assert(static_cast<std::string>(api_doc["error"]["code"]) == expected_code);
  assert(report.find("\"points\"") == std::string::npos);
  const auto cli_report = root / (stem + "-cli-report.json");
  const std::string command = "\"" + cli.string() + "\" reconstruct-sparse \"" +
                              project_path.string() + "\" > \"" + cli_report.string() + "\"";
  assert(std::system(command.c_str()) != 0);
  const auto cli_json = read_text(cli_report);
  auto cli_doc = parse(cli_json);
  assert(static_cast<int>(cli_doc["ok"]) == 0);
  assert(static_cast<std::string>(cli_doc["error"]["code"]) == expected_code);
}
} // namespace

int main(int argc, char **argv) {
  assert(argc == 3);
  const fs::path root = argv[1], cli = argv[2];
  fs::create_directories(root);
  const auto dictionary = cv::aruco::getPredefinedDictionary(cv::aruco::DICT_4X4_50);
  std::vector<std::string> paths, masks;
  std::ostringstream truth;
  truth << "{\"description\":\"R02 synthetic metric scene, all lengths mm\","
           "\"objectPlanes\":[{\"x\":[75,145],\"y\":[75,225],\"z\":-45},"
           "{\"x\":[155,230],\"y\":[75,225],\"z\":-85}],\"views\":[";
  std::vector<cv::Mat> safe_masks;
  std::vector<cv::Mat> expected_rotations, expected_translations;
  for (int i = 0; i < 3; ++i) {
    const double camera_x = 110 + 40 * i;
    cv::Mat center = (cv::Mat_<double>(3, 1) << camera_x, 150 + 5 * i, -700);
    cv::Mat rvec = (cv::Mat_<double>(3, 1) << -0.04 + 0.025 * i, 0.035 - 0.02 * i, 0.005 * i);
    cv::Mat R;
    cv::Rodrigues(rvec, R);
    cv::Mat tvec = -R * center;
    expected_rotations.push_back(R);
    expected_translations.push_back(tvec);
    cv::Mat image(height, width, CV_8UC1, cv::Scalar(110));
    composite(image, texture(950, 950, 18001),
              projection({-50, -50, 350, 350, 12}, rvec, tvec));
    for (size_t m = 0; m < marker_origins.size(); ++m) {
      const auto [x, y] = marker_origins[m];
      cv::Mat white(330, 330, CV_8UC1, cv::Scalar(255));
      composite(image, white, projection({x - 5, y - 5, x + 45, y + 45, 0}, rvec, tvec));
      cv::Mat marker;
      cv::aruco::generateImageMarker(dictionary, static_cast<int>(m + 1), 300, marker, 1);
      composite(image, marker, projection({x, y, x + 40, y + 40, 0}, rvec, tvec), nullptr,
                cv::INTER_NEAREST);
    }
    cv::Mat object_mask(height, width, CV_8UC1, cv::Scalar(0));
    for (size_t p = 0; p < panels.size(); ++p) {
      composite(image, texture(750, 350, 53000 + p * 701),
                projection(panels[p], rvec, tvec), &object_mask);
    }
    cv::threshold(object_mask, object_mask, 127, 255, cv::THRESH_BINARY);
    cv::Mat safe;
    cv::erode(object_mask, safe, cv::getStructuringElement(cv::MORPH_ELLIPSE, {65, 65}),
              {-1, -1}, 1, cv::BORDER_CONSTANT, cv::Scalar(0));
    safe_masks.push_back(safe);
    const auto filename = "view-" + std::to_string(i + 1) + ".png";
    const auto maskname = "view-" + std::to_string(i + 1) + "-object-mask.png";
    assert(cv::imwrite((root / filename).string(), image));
    assert(cv::imwrite((root / maskname).string(), object_mask));
    paths.push_back(filename);
    masks.push_back(maskname);
    if (i) truth << ',';
    truth << "{\"imageId\":\"view-" << i + 1 << "\",\"cameraCenterMm\":["
          << center.at<double>(0) << ',' << center.at<double>(1) << ',' << center.at<double>(2)
          << "],\"rotation\":[";
    for (int k = 0; k < 9; ++k) { if (k) truth << ','; truth << R.at<double>(k / 3, k % 3); }
    truth << "],\"translationMm\":[" << tvec.at<double>(0) << ',' << tvec.at<double>(1)
          << ',' << tvec.at<double>(2) << "]}";
  }
  truth << "]}";
  write_text(root / "truth.json", truth.str());
  const auto project = project_json(paths, masks);
  write_text(root / "project.json", project);
  const auto report = api(root / "project.json", CRISPMESH_OK);
  write_text(root / "sparse-report.json", report);
  auto doc = parse(report);
  assert(static_cast<int>(doc["ok"]) == 1);
  assert(static_cast<int>(doc["sparseSchemaVersion"]) == 1);
  assert(static_cast<std::string>(doc["projectId"]) == "synthetic-sparse-r02");
  assert(static_cast<std::string>(doc["units"]) == "mm");
  assert(static_cast<std::string>(doc["coordinateFrame"]) == "board");
  assert(static_cast<std::string>(doc["backend"]["name"]) == "OpenCV");
  assert(static_cast<std::string>(doc["backend"]["feature"]) == "ORB");
  auto views = doc["views"];
  assert(views.isSeq() && views.size() == 3);
  for (int i = 0; i < 3; ++i) {
    assert(static_cast<std::string>(views[i]["imageId"]) == "view-" + std::to_string(i + 1));
    assert(static_cast<std::string>(views[i]["maskPath"]) == masks[i]);
    assert(static_cast<int>(views[i]["featureCount"]) > 20);
    const auto actual_t = xyz(views[i]["translationMm"]);
    auto actual_R = views[i]["rotation"];
    assert(actual_R.isSeq() && actual_R.size() == 9);
    double rotation_squared = 0;
    for (int k = 0; k < 3; ++k) {
      assert(std::abs(actual_t[k] - expected_translations[i].at<double>(k)) < 7);
    }
    for (int k = 0; k < 9; ++k) {
      const double difference = number(actual_R[k]) - expected_rotations[i].at<double>(k / 3, k % 3);
      rotation_squared += difference * difference;
    }
    assert(std::sqrt(rotation_squared) < 0.12);
  }
  auto points = doc["points"];
  assert(points.isSeq() && points.size() >= 25);
  assert(static_cast<int>(doc["statistics"]["acceptedTracks"]) == static_cast<int>(points.size()));
  int near = 0, each_depth[2] = {0, 0}, multiview = 0;
  std::vector<double> depth_errors, maximum_reprojection_errors;
  for (const auto &point : points) {
    const auto p = xyz(point["positionMm"]);
    for (double v : p) assert(std::isfinite(v));
    depth_errors.push_back(std::min(std::abs(p[2] - panels[0].z),
                                    std::abs(p[2] - panels[1].z)));
    maximum_reprojection_errors.push_back(number(point["maxReprojectionErrorPx"]));
    if (near_panel(p)) ++near;
    if (std::abs(p[2] - panels[0].z) < 13) ++each_depth[0];
    if (std::abs(p[2] - panels[1].z) < 13) ++each_depth[1];
    assert(number(point["maxReprojectionErrorPx"]) <= 2.05);
    assert(number(point["rmsReprojectionErrorPx"]) <= 2.05);
    assert(number(point["minParallaxDeg"]) >= 0.95);
    auto observations = point["observations"];
    assert(observations.isSeq() && observations.size() >= 2);
    if (observations.size() == 3) ++multiview;
    bool seen[3] = {false, false, false};
    for (const auto &obs : observations) {
      std::string id = obs["imageId"];
      assert(id.size() == 6 && id.substr(0, 5) == "view-");
      int i = id[5] - '1';
      assert(i >= 0 && i < 3 && !seen[i]);
      seen[i] = true;
      auto pixel = obs["pixel"];
      assert(pixel.isSeq() && pixel.size() == 2);
      int x = cvRound(number(pixel[0])), y = cvRound(number(pixel[1]));
      assert(x >= 0 && x < width && y >= 0 && y < height);
      assert(safe_masks[i].at<uchar>(y, x) == 255);
    }
  }
  if (near < static_cast<int>(points.size() * 0.90) || each_depth[0] < 5 || each_depth[1] < 5) {
    std::cerr << "Sparse geometry: " << points.size() << " points, " << near
              << " near object, depths " << each_depth[0] << '/' << each_depth[1] << '\n';
    std::abort();
  }
  assert(multiview >= 10);
  std::sort(depth_errors.begin(), depth_errors.end());
  std::sort(maximum_reprojection_errors.begin(), maximum_reprojection_errors.end());
  auto percentile = [](const std::vector<double> &values, double fraction) {
    return values[static_cast<size_t>(std::floor(fraction * (values.size() - 1)))];
  };
  const double depth_median = percentile(depth_errors, 0.5);
  const double depth_p95 = percentile(depth_errors, 0.95);
  assert(depth_median < 3 && depth_p95 < 8);
  std::ostringstream quality;
  quality << "{\"description\":\"R02 synthetic baseline; rasterized texture and estimated marker poses, not real-photo accuracy\","
          << "\"pointCount\":" << points.size() << ",\"nearKnownPanels\":" << near
          << ",\"threeViewTracks\":" << multiview << ",\"depthErrorMm\":{\"median\":"
          << depth_median << ",\"p95\":" << depth_p95 << ",\"max\":" << depth_errors.back()
          << "},\"maxReprojectionErrorPx\":{\"median\":"
          << percentile(maximum_reprojection_errors, 0.5) << ",\"p95\":"
          << percentile(maximum_reprojection_errors, 0.95) << ",\"max\":"
          << maximum_reprojection_errors.back() << "}}";
  write_text(root / "quality-report.json", quality.str());
  const std::string cli_command = "\"" + cli.string() + "\" reconstruct-sparse \"" +
                                  (root / "project.json").string() + "\" > \"" +
                                  (root / "sparse-cli-report.json").string() + "\"";
  assert(std::system(cli_command.c_str()) == 0);
  auto cli_doc = parse(read_text(root / "sparse-cli-report.json"));
  assert(static_cast<int>(cli_doc["ok"]) == 1);
  assert(cli_doc["points"].size() == points.size());

  auto missing = masks;
  missing[1] = "missing-mask.png";
  expect_failure(root, cli, "missing-mask", project_json(paths, missing), "missing_mask");
  cv::Mat wrong_size(height / 2, width / 2, CV_8UC1, cv::Scalar(255));
  assert(cv::imwrite((root / "wrong-size-mask.png").string(), wrong_size));
  auto wrong = masks;
  wrong[0] = "wrong-size-mask.png";
  expect_failure(root, cli, "wrong-size-mask", project_json(paths, wrong), "mask_dimensions");
  cv::Mat nonbinary = cv::imread((root / masks[0]).string(), cv::IMREAD_GRAYSCALE);
  assert(!nonbinary.empty());
  nonbinary.at<uchar>(height / 2, width / 2) = 127;
  assert(cv::imwrite((root / "nonbinary-mask.png").string(), nonbinary));
  auto bad_binary = masks;
  bad_binary[0] = "nonbinary-mask.png";
  expect_failure(root, cli, "nonbinary-mask", project_json(paths, bad_binary), "invalid_mask");
  const fs::path outside_mask = root.parent_path() / "outside-synthetic-sparse-mask.png";
  fs::copy_file(root / masks[0], outside_mask, fs::copy_options::overwrite_existing);
  const fs::path escape_mask = root / "escaped-mask.png";
  std::error_code symlink_error;
  fs::remove(escape_mask, symlink_error);
  fs::create_symlink(fs::absolute(outside_mask), escape_mask, symlink_error);
  assert(!symlink_error);
  auto escaped = masks;
  escaped[0] = "escaped-mask.png";
  expect_failure(root, cli, "escaped-mask", project_json(paths, escaped), "path_escape");
  cv::Mat empty(height, width, CV_8UC1, cv::Scalar(0));
  assert(cv::imwrite((root / "empty-mask.png").string(), empty));
  auto blank = std::vector<std::string>(masks.size(), "empty-mask.png");
  expect_failure(root, cli, "empty-mask", project_json(paths, blank), "no_features");
  std::vector<std::string> duplicate_paths{paths[0], paths[0], paths[0]};
  std::vector<std::string> duplicate_masks{masks[0], masks[0], masks[0]};
  expect_failure(root, cli, "zero-baseline", project_json(duplicate_paths, duplicate_masks,
                                                           "synthetic-zero-baseline"), "degenerate_baseline");
  std::cout << "R02 synthetic metric scene passed: " << points.size() << " points, " << near
            << " near known panels, " << multiview << " three-view tracks. Artifacts: " << root << '\n';
}
