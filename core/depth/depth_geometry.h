#pragma once

#include <opencv2/core.hpp>
#include <string>
#include <vector>

namespace crispmesh::depth {

// X_camera = rotation * X_object + translation_mm.
struct Pose {
  cv::Matx33d rotation = cv::Matx33d::eye();
  cv::Vec3d translation_mm{0, 0, 0};
};

struct Camera {
  cv::Matx33d intrinsic = cv::Matx33d::eye();
  cv::Mat distortion;  // Empty, or OpenCV radtan coefficients (4, 5, or 8).
  cv::Size image_size;
  Pose pose;
};

enum class Status { ok, invalid_input, degenerate_pose, unsupported_axis, invalid_sample };
struct Result {
  Status status = Status::ok;
  std::string reason;
  explicit operator bool() const { return status == Status::ok; }
};

enum class Axis { horizontal, vertical };
struct RectifiedPair {
  cv::Matx33d relative_rotation, rectification1, rectification2;
  cv::Vec3d relative_translation_mm;
  cv::Matx34d projection1, projection2;
  cv::Matx44d q;
  cv::Mat map1x, map1y, map2x, map2y;
  Axis axis = Axis::horizontal;
  double signed_baseline_mm = 0;
  double disparity_min_px = 0, disparity_max_px = 0;
};

// Bounds are along rectified camera-1 Z, and must be supplied in millimetres.
Result rectify(const Camera& first, const Camera& second, double near_mm,
               double far_mm, RectifiedPair& output);

// Pixel coordinates refer to the original, distorted input images.
Result triangulate_object(const Camera& first, const Camera& second,
                          cv::Point2d pixel1, cv::Point2d pixel2,
                          cv::Vec3d& object_mm);

// A sample is an independently supplied object-frame candidate, e.g. from a
// triangulated correspondence. Track IDs prevent fusing adjacent surfaces.
struct Candidate {
  cv::Vec3d object_mm;
  double confidence = 0;
  int track_id = -1;
};
struct DepthView {
  Camera camera;
  cv::Mat depth_mm;  // CV_32FC1 axial Z in this camera's frame.
  cv::Mat object_mask;  // CV_8UC1, 0 excludes; nearest-pixel lookup.
};
struct FusedPoint {
  int track_id = -1;
  cv::Vec3d object_mm;
  double confidence = 0;
  int support_views = 0;
};
struct FusionStats {
  size_t input_candidates = 0, rejected_candidates = 0;
  size_t accepted_candidates = 0, discarded_by_cluster = 0;
  size_t rejected_centroids = 0, output_points = 0;
};

// A candidate needs agreement with at least two distinct masked depth views.
// An observed nearer surface occludes the candidate in that view; a farther
// observed surface vetoes it. No spatial smoothing or proximity-only merging.
Result fuse_tracks(const std::vector<Candidate>& candidates,
                   const std::vector<DepthView>& views, double tolerance_mm,
                   std::vector<FusedPoint>& points, FusionStats& stats);

}  // namespace crispmesh::depth
