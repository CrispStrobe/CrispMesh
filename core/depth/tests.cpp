#include "depth_geometry.h"
#include <opencv2/calib3d.hpp>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <iostream>
#include <limits>
#include <stdexcept>

using namespace crispmesh::depth;
namespace {
void check(bool ok,const char* message) { if (!ok) throw std::runtime_error(message); }
Camera camera(cv::Vec3d center, cv::Matx33d r=cv::Matx33d::eye()) {
  Camera c;
  c.image_size={640,480};
  c.intrinsic=cv::Matx33d(800,0,320, 0,810,240, 0,0,1);
  c.distortion=(cv::Mat_<double>(1,5)<<0.018,-0.003,0.001,-0.001,0.0002);
  c.pose={r,-r*center};
  return c;
}
cv::Point2d pixel(const Camera& c, cv::Vec3d object) {
  cv::Vec3d x=c.pose.rotation*object+c.pose.translation_mm;
  std::vector<cv::Point2d> p;
  cv::projectPoints(std::vector<cv::Point3d>{cv::Point3d(x)},cv::Vec3d(0,0,0),
                    cv::Vec3d(0,0,0),cv::Mat(c.intrinsic),c.distortion,p);
  return p[0];
}
DepthView view(const Camera& c) {
  return {c,cv::Mat(c.image_size,CV_32FC1,cv::Scalar(0)),
          cv::Mat(c.image_size,CV_8UC1,cv::Scalar(0))};
}
void mark(DepthView& v, cv::Vec3d p, float depth_override=0) {
  auto q=pixel(v.camera,p); int x=cvRound(q.x),y=cvRound(q.y);
  check(x>=0 && y>=0 && x<v.depth_mm.cols && y<v.depth_mm.rows,"fixture outside frame");
  v.object_mask.at<unsigned char>(y,x)=255;
  v.depth_mm.at<float>(y,x)=depth_override>0 ? depth_override :
      float((v.camera.pose.rotation*p+v.camera.pose.translation_mm)[2]);
}
}  // namespace

int main() {
  try {
    const auto started=std::chrono::steady_clock::now();
    auto a=camera({0,0,0}), b=camera({40,0,0}), c=camera({0,35,0});
    // Nontrivial object-to-camera rotation and translation must compose as
    // R_b R_a^T, t_b - R_b R_a^T t_a, independently checked on world points.
    const double angle=0.09;
    cv::Matx33d yaw(std::cos(angle),0,std::sin(angle), 0,1,0,
                   -std::sin(angle),0,std::cos(angle));
    auto turned=camera({40,0,0},yaw);
    cv::Matx33d pitch(1,0,0, 0,std::cos(0.06),-std::sin(0.06),
                     0,std::sin(0.06),std::cos(0.06));
    auto first_rotated=camera({12,-8,5},pitch);
    auto second_rotated=camera({52,4,5},yaw*pitch);
    RectifiedPair nonidentity;
    check(bool(rectify(first_rotated,second_rotated,350,750,nonidentity)),
          "both nonidentity poses rectify");
    check(cv::norm(-first_rotated.pose.rotation.t()*first_rotated.pose.translation_mm-
                   cv::Vec3d(12,-8,5))<1e-10,"camera center inverse pose");
    check(cv::norm(nonidentity.relative_rotation-
                   second_rotated.pose.rotation*first_rotated.pose.rotation.t())<1e-10,
          "relative rotation");
    check(cv::norm(nonidentity.relative_translation_mm-
                   (second_rotated.pose.translation_mm-
                    nonidentity.relative_rotation*first_rotated.pose.translation_mm))<1e-10,
          "relative translation");
    RectifiedPair h,v,rotated;
    check(bool(rectify(a,b,350,750,h)),"horizontal rectification");
    check(h.axis==Axis::horizontal,"horizontal axis");
    check(bool(rectify(a,c,350,750,v)),"vertical rectification");
    check(v.axis==Axis::vertical,"vertical axis");
    check(bool(rectify(a,turned,350,750,rotated)),"rotated rectification");
    cv::Vec3d truth(17,-13,500);
    auto pa=pixel(a,truth), pb=pixel(b,truth), pc=pixel(c,truth);
    check(std::abs(pa.x-pb.x-64)<0.1,"analytic horizontal parallax in pixels");
    check(std::abs(pa.y-pc.y-56.7)<0.1,"analytic vertical parallax in pixels");
    check(std::abs(cv::norm(rotated.relative_rotation*a.pose.rotation*truth+
             rotated.relative_translation_mm - (turned.pose.rotation*truth+turned.pose.translation_mm)))<1e-8,
          "relative transform composition");
    for (const auto& other:{b,c,turned}) {
      cv::Vec3d recovered;
      check(bool(triangulate_object(a,other,pa,pixel(other,truth),recovered)),"triangulation");
      check(cv::norm(recovered-truth)<1e-5,"object-frame metric reconstruction");
    }
    cv::Vec3d nonidentity_recovered;
    check(bool(triangulate_object(first_rotated,second_rotated,pixel(first_rotated,truth),
                                  pixel(second_rotated,truth),nonidentity_recovered)),
          "nonidentity triangulation");
    check(cv::norm(nonidentity_recovered-truth)<1e-5,"nonidentity object frame");
    check(h.disparity_min_px<=64 && h.disparity_max_px>=64,"horizontal depth search bound");
    check(v.disparity_min_px<=56.7 && v.disparity_max_px>=56.7,"vertical depth search bound");
    check(rectify(a,a,350,750,h).status==Status::degenerate_pose,"degenerate pose failure");
    check(rectify(a,b,0,750,h).status==Status::invalid_input,"invalid depth failure");
    Camera bad=a;
    bad.intrinsic(0,2)=std::numeric_limits<double>::quiet_NaN();
    check(rectify(bad,b,350,750,h).status==Status::invalid_input,"nonfinite intrinsic failure");
    bad=a; bad.pose.rotation(1,1)=std::numeric_limits<double>::quiet_NaN();
    check(rectify(bad,b,350,750,h).status==Status::invalid_input,"nonfinite rotation failure");
    bad=a; bad.distortion=bad.distortion.clone();
    bad.distortion.at<double>(0,0)=std::numeric_limits<double>::infinity();
    check(rectify(bad,b,350,750,h).status==Status::invalid_input,"nonfinite distortion failure");
    std::vector<DepthView> views{view(a),view(b),view(c)};
    cv::Vec3d thin1(0,0,500), thin2(0.3,0,500.4), background(25,25,500);
    for (auto& d:views) { mark(d,thin1); mark(d,thin2); }
    // A separate track never merges solely because it is within tolerance.
    std::vector<Candidate> candidates{{thin1,1,10},{thin1+cv::Vec3d(0.1,0,0),2,10},
                                      {thin2,1,11},{background,1,12},
                                      {{0,0,-100},1,13},{{60,0,500},1,14}};
    // Add an outlier whose projected depth conflicts with the surface.
    mark(views[0],{60,0,500},550);
    mark(views[1],{60,0,500});
    std::vector<FusedPoint> fused; FusionStats stats;
    check(bool(fuse_tracks(candidates,views,1.0,fused,stats)),"fusion call");
    check(fused.size()==2 && stats.rejected_candidates==3,"fusion mask, behind and conflict rejection");
    check(fused[0].track_id==10 && std::abs(fused[0].object_mm[0]-0.0666667)<1e-5,
          "confidence weighted fusion");
    check(fused[1].track_id==11 && cv::norm(fused[1].object_mm-thin2)<1e-9,
          "thin neighbor remains distinct");
    check(fused[0].support_views==3,"three-view consistency");
    const FusionStats main_stats=stats;
    auto occluded=views;
    occluded[2].depth_mm=views[2].depth_mm.clone();
    mark(occluded[2],thin1,450);
    check(bool(fuse_tracks({{thin1,1,30}},occluded,1.0,fused,stats)) &&
          fused.size()==1 && fused[0].support_views==2,
          "nearer observed surface only occludes one view");
    check(bool(fuse_tracks({{thin1,1,31}},std::vector<DepthView>{views[0],views[0]},
                           1.0,fused,stats)) && fused.empty(),
          "duplicate camera center is not multiview support");
    std::vector<DepthView> islands{view(a),view(b),view(c)};
    for (auto& d:islands) { mark(d,{5,0,500}); mark(d,{7,0,500}); }
    std::vector<Candidate> bridge{{{5,0,500},1,20},{{7,0,500},1,20}};
    check(bool(fuse_tracks(bridge,islands,3.0,fused,stats)),"disconnected mask fusion call");
    check(fused.empty() && stats.accepted_candidates==2 && stats.rejected_centroids==1,
          "weighted centroid cannot bridge excluded mask");
    auto elapsed=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-started).count();
    std::cout<<"{\"ok\":true,\"units\":\"mm\",\"views\":3,\"inputCandidates\":"
             <<main_stats.input_candidates<<",\"acceptedCandidates\":"<<main_stats.accepted_candidates
             <<",\"rejectedCandidates\":"<<main_stats.rejected_candidates
             <<",\"fusedTracks\":"<<main_stats.output_points<<",\"elapsedMs\":"<<elapsed<<"}\n";
    return EXIT_SUCCESS;
  } catch (const std::exception& e) {
    std::cerr<<"depth geometry test failed: "<<e.what()<<'\n'; return EXIT_FAILURE;
  }
}
