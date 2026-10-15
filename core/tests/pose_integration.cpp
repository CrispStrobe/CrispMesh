#ifdef NDEBUG
#undef NDEBUG
#endif
#include "crispmesh/core.h"

#include <opencv2/calib3d.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/objdetect/aruco_detector.hpp>

#include <array>
#include <cassert>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

namespace fs = std::filesystem;

static void write_text(const fs::path &path, const std::string &s) {
  std::ofstream file(path, std::ios::binary);
  file << s;
  assert(file.good());
}
static std::string read_text(const fs::path &path) {
  std::ifstream file(path, std::ios::binary);
  return {std::istreambuf_iterator<char>(file), std::istreambuf_iterator<char>()};
}
static std::string run_api(const fs::path &project, crispmesh_status expected) {
  size_t size = 0;
  char *output = nullptr;
  auto status = crispmesh_estimate_poses_alloc(project.string().c_str(), &output, &size);
  if (status != expected) {
    std::cerr << "Unexpected status " << status << ": " << (output ? output : "null") << '\n';
    std::abort();
  }
  std::string result = output;
  crispmesh_free(output);
  return result;
}
static std::string project_json(const std::string &image_path, int width = 1280, int height = 960) {
  return "{\"schemaVersion\":1,\"id\":\"synthetic-known-pose\",\"name\":\"Synthetic pose fixture\","
         "\"units\":\"mm\",\"images\":[{\"id\":\"view-1\",\"path\":\"" + image_path +
         "\"}],\"calibration\":{\"width\":" + std::to_string(width) + ",\"height\":" +
         std::to_string(height) + ",\"fx\":1100,\"fy\":1080,\"cx\":640,\"cy\":480,"
         "\"distortionModel\":\"opencv-radtan\",\"distortion\":[0,0,0,0]},"
         "\"board\":{\"type\":\"aruco\",\"dictionary\":\"DICT_4X4_50\",\"markerSizeMm\":40,"
         "\"rotatesWithObject\":true,\"markers\":["
         "{\"id\":1,\"cornersMm\":[[0,0,0],[40,0,0],[40,40,0],[0,40,0]]},"
         "{\"id\":2,\"cornersMm\":[[70,0,0],[110,0,0],[110,40,0],[70,40,0]]},"
         "{\"id\":3,\"cornersMm\":[[0,70,0],[40,70,0],[40,110,0],[0,110,0]]},"
         "{\"id\":4,\"cornersMm\":[[70,70,0],[110,70,0],[110,110,0],[70,110,0]]}]},"
         "\"stages\":{}}";
}
static std::vector<double> numbers_after(const std::string &text, const std::string &key, size_t count) {
  size_t p = text.find(key);
  assert(p != std::string::npos);
  p = text.find('[', p);
  assert(p != std::string::npos);
  ++p;
  std::vector<double> result;
  while (result.size() < count) {
    char *end = nullptr;
    double number = std::strtod(text.c_str() + p, &end);
    assert(end != text.c_str() + p);
    result.push_back(number);
    p = static_cast<size_t>(end - text.c_str());
    if (text[p] == ',') ++p;
  }
  return result;
}

int main(int argc, char **argv) {
  assert(argc == 3);
  fs::path root = argv[1], cli = argv[2];
  fs::create_directories(root);
  const auto dictionary = cv::aruco::getPredefinedDictionary(cv::aruco::DICT_4X4_50);
  cv::Mat image(960,1280,CV_8UC1,cv::Scalar(255));
  cv::Mat K = (cv::Mat_<double>(3,3) << 1100,0,640,0,1080,480,0,0,1);
  cv::Mat rvec = (cv::Mat_<double>(3,1) << -0.32,0.27,0.08);
  cv::Mat tvec = (cv::Mat_<double>(3,1) << -55,-55,530);
  const std::array<int,4> ids{1,2,3,4};
  for (size_t i = 0; i < ids.size(); ++i) {
    double x = i % 2 ? 70 : 0, y = i / 2 ? 70 : 0;
    std::vector<cv::Point3f> object{{float(x),float(y),0},{float(x+40),float(y),0},
                                     {float(x+40),float(y+40),0},{float(x),float(y+40),0}};
    std::vector<cv::Point2f> projected;
    cv::projectPoints(object,rvec,tvec,K,cv::Mat(),projected);
    cv::Mat marker;
    cv::aruco::generateImageMarker(dictionary,ids[i],240,marker,1);
    std::vector<cv::Point2f> source{{0,0},{239,0},{239,239},{0,239}};
    cv::Mat warp = cv::getPerspectiveTransform(source,projected);
    cv::Mat rendered(image.size(),CV_8UC1,cv::Scalar(255));
    cv::warpPerspective(marker,rendered,warp,image.size(),cv::INTER_NEAREST,
                        cv::BORDER_CONSTANT,cv::Scalar(255));
    cv::min(image,rendered,image);
  }
  assert(cv::imwrite((root / "synthetic-board.png").string(),image));
  write_text(root / "project.json",project_json("synthetic-board.png"));
  auto result = run_api(root / "project.json", CRISPMESH_OK);
  assert(result.find("\"detectedMarkerIds\"") != std::string::npos);
  assert(result.find("\"rmsReprojectionErrorPx\"") != std::string::npos);
  auto translation = numbers_after(result,"\"translationMm\"",3);
  for (int i = 0; i < 3; ++i) assert(std::abs(translation[i] - tvec.at<double>(i)) < 5);
  auto rotation = numbers_after(result,"\"rotation\"",9);
  cv::Mat expected;
  cv::Rodrigues(rvec,expected);
  double squared = 0;
  for (int i = 0; i < 9; ++i) squared += std::pow(rotation[i]-expected.at<double>(i/3,i%3),2);
  assert(std::sqrt(squared) < 0.1);
  std::string cli_command = "\"" + cli.string() + "\" estimate-poses \"" +
                            (root / "project.json").string() + "\" > \"" +
                            (root / "pose-report.json").string() + "\"";
  assert(std::system(cli_command.c_str()) == 0);
  assert(read_text(root / "pose-report.json").find("\"ok\":true") != std::string::npos);

  assert(cv::imwrite((root / "synthetic-board.jpg").string(),image,
                     {cv::IMWRITE_JPEG_QUALITY,95}));
  write_text(root / "jpeg.json",project_json("synthetic-board.jpg"));
  auto jpeg_result = run_api(root / "jpeg.json",CRISPMESH_OK);
  auto jpeg_translation = numbers_after(jpeg_result,"\"translationMm\"",3);
  for (int i = 0; i < 3; ++i) assert(std::abs(jpeg_translation[i] - tvec.at<double>(i)) < 5);

  cv::Mat blank(image.size(),CV_8UC1,cv::Scalar(255));
  assert(cv::imwrite((root / "blank.png").string(),blank));
  write_text(root / "missing.json",project_json("blank.png"));
  assert(run_api(root / "missing.json",CRISPMESH_INVALID_PROJECT).find("insufficient_markers") != std::string::npos);
  std::string mixed = project_json("synthetic-board.png");
  size_t image_end = mixed.find("}],\"calibration\"");
  assert(image_end != std::string::npos);
  mixed.replace(image_end,2,"},{\"id\":\"view-2\",\"path\":\"blank.png\"}]");
  write_text(root / "mixed.json",mixed);
  auto mixed_result = run_api(root / "mixed.json",CRISPMESH_INVALID_PROJECT);
  assert(mixed_result.find("insufficient_markers") != std::string::npos);
  assert(mixed_result.find("\"poses\"") == std::string::npos);
  std::string mixed_command = "\"" + cli.string() + "\" estimate-poses \"" +
                              (root / "mixed.json").string() + "\" > \"" +
                              (root / "mixed-report.json").string() + "\"";
  assert(std::system(mixed_command.c_str()) != 0);
  auto mixed_cli = read_text(root / "mixed-report.json");
  assert(mixed_cli.find("\"ok\":false") != std::string::npos);
  assert(mixed_cli.find("\"poses\"") == std::string::npos);
  write_text(root / "dimensions.json",project_json("synthetic-board.png",640,480));
  assert(run_api(root / "dimensions.json",CRISPMESH_INVALID_PROJECT).find("image_dimensions") != std::string::npos);
  std::string huge = project_json("synthetic-board.png");
  size_t width_at = huge.find("\"width\":1280");
  assert(width_at != std::string::npos);
  huge.replace(width_at,std::string("\"width\":1280").size(),"\"width\":1e100");
  write_text(root / "huge-width.json",huge);
  assert(run_api(root / "huge-width.json",CRISPMESH_INVALID_PROJECT).find("resource_limit") != std::string::npos);
  fs::copy_file(root / "synthetic-board.png",root.parent_path() / "outside-synthetic-board.png",
                fs::copy_options::overwrite_existing);
  std::error_code ec;
  fs::remove(root / "escape.png",ec);
  fs::create_symlink(root.parent_path() / "outside-synthetic-board.png",root / "escape.png");
  write_text(root / "escape.json",project_json("escape.png"));
  assert(run_api(root / "escape.json",CRISPMESH_INVALID_PROJECT).find("path_escape") != std::string::npos);
  std::string oversized("\x89PNG\r\n\x1a\n",8);
  oversized += std::string("\0\0\0\rIHDR",8);
  oversized += std::string("\0\0\x27\x10\0\0\x27\x10",8);
  write_text(root / "oversized.png",oversized);
  write_text(root / "oversized.json",project_json("oversized.png",10000,10000));
  assert(run_api(root / "oversized.json",CRISPMESH_INVALID_PROJECT).find("resource_limit") != std::string::npos);
  std::cout << "Synthetic detector-to-PnP fixture and failure cases passed; artifacts: " << root << '\n';
}
