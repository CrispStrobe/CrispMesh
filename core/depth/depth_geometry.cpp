#include "depth_geometry.h"

#include <opencv2/calib3d.hpp>
#include <opencv2/imgproc.hpp>
#include <algorithm>
#include <cmath>
#include <map>
#include <set>

namespace crispmesh::depth {
namespace {
bool finite(const cv::Vec3d& v) {
  return std::isfinite(v[0]) && std::isfinite(v[1]) && std::isfinite(v[2]);
}
bool valid(const Camera& c) {
  if (c.image_size.width <= 0 || c.image_size.height <= 0 ||
      c.image_size.width > 8192 || c.image_size.height > 8192 ||
      int64_t(c.image_size.width)*c.image_size.height > 8000000 ||
      c.intrinsic(0, 0) < 1e-6 || c.intrinsic(1, 1) < 1e-6 ||
      c.intrinsic(0, 0) > 1e7 || c.intrinsic(1, 1) > 1e7 ||
      !finite(c.pose.translation_mm)) return false;
  for (double x:c.intrinsic.val) if (!std::isfinite(x)) return false;
  for (double x:c.pose.rotation.val) if (!std::isfinite(x)) return false;
  if (std::abs(c.intrinsic(2,0))>1e-12 || std::abs(c.intrinsic(2,1))>1e-12 ||
      std::abs(c.intrinsic(2,2)-1)>1e-12 || std::abs(c.intrinsic(0,1))>1e-12 ||
      std::abs(c.intrinsic(1,0))>1e-12) return false;
  const auto r = c.pose.rotation;
  if (cv::norm(cv::Mat(r * r.t() - cv::Matx33d::eye())) > 1e-5 ||
      std::abs(cv::determinant(cv::Mat(r)) - 1) > 1e-5) return false;
  if (!c.distortion.empty() && (!(c.distortion.total() == 4 || c.distortion.total() == 5 || c.distortion.total() == 8) ||
      c.distortion.channels()!=1 || (c.distortion.rows!=1 && c.distortion.cols!=1) ||
      (c.distortion.depth()!=CV_64F && c.distortion.depth()!=CV_32F))) return false;
  if (!c.distortion.empty()) {
    cv::Mat d; c.distortion.convertTo(d,CV_64F);
    for (size_t i=0;i<d.total();++i)
      if (!std::isfinite(d.ptr<double>()[i]) || std::abs(d.ptr<double>()[i])>1e4) return false;
  }
  return true;
}
cv::Matx34d matrix34(const cv::Mat& m) {
  cv::Matx34d out;
  for (int y=0;y<3;++y) for (int x=0;x<4;++x) out(y,x)=m.at<double>(y,x);
  return out;
}
cv::Matx33d matrix33(const cv::Mat& m) {
  cv::Matx33d out;
  for (int y=0;y<3;++y) for (int x=0;x<3;++x) out(y,x)=m.at<double>(y,x);
  return out;
}
cv::Matx44d matrix44(const cv::Mat& m) {
  cv::Matx44d out;
  for (int y=0;y<4;++y) for (int x=0;x<4;++x) out(y,x)=m.at<double>(y,x);
  return out;
}
cv::Vec3d camera_point(const Camera& c, const cv::Vec3d& x) {
  return c.pose.rotation*x + c.pose.translation_mm;
}
}  // namespace

Result rectify(const Camera& a, const Camera& b, double near_mm,
               double far_mm, RectifiedPair& out) {
  if (!valid(a) || !valid(b) || a.image_size != b.image_size ||
      !std::isfinite(near_mm) || !std::isfinite(far_mm) || near_mm <= 0 ||
      far_mm <= near_mm || far_mm > 1e9)
    return {Status::invalid_input, "invalid calibration, image size, or depth interval"};
  const cv::Matx33d r = b.pose.rotation * a.pose.rotation.t();
  const cv::Vec3d t = b.pose.translation_mm - r*a.pose.translation_mm;
  if (cv::norm(t) < 1e-3) return {Status::degenerate_pose, "baseline below 0.001 mm"};
  cv::Mat R1, R2, P1, P2, Q;
  try {
    cv::stereoRectify(cv::Mat(a.intrinsic), a.distortion, cv::Mat(b.intrinsic), b.distortion,
                      a.image_size, cv::Mat(r), cv::Mat(t), R1, R2, P1, P2, Q,
                      cv::CALIB_ZERO_DISPARITY, 0, a.image_size);
  } catch (const cv::Exception& e) { return {Status::invalid_input, e.what()}; }
  if (!cv::checkRange(R1) || !cv::checkRange(R2) || !cv::checkRange(P1) ||
      !cv::checkRange(P2) || !cv::checkRange(Q) ||
      P1.at<double>(0,0)<=0 || P1.at<double>(1,1)<=0 ||
      P2.at<double>(0,0)<=0 || P2.at<double>(1,1)<=0)
    return {Status::invalid_input,"nonfinite or invalid rectification matrices"};
  const double tx=P2.at<double>(0,3), ty=P2.at<double>(1,3);
  const double scale=std::max(std::abs(tx), std::abs(ty));
  if (scale < 1e-9) return {Status::degenerate_pose, "rectified baseline is zero"};
  if (std::min(std::abs(tx), std::abs(ty)) > 1e-6*scale)
    return {Status::unsupported_axis, "rectified epipolar translation is not axis aligned"};
  out.relative_rotation=r; out.relative_translation_mm=t;
  out.rectification1=matrix33(R1); out.rectification2=matrix33(R2);
  out.projection1=matrix34(P1); out.projection2=matrix34(P2); out.q=matrix44(Q);
  out.axis=std::abs(tx)>std::abs(ty) ? Axis::horizontal : Axis::vertical;
  const int i=out.axis==Axis::horizontal ? 0 : 1;
  out.signed_baseline_mm=P2.at<double>(i,3)/P2.at<double>(i,i);
  const double offset=P1.at<double>(i,2)-P2.at<double>(i,2);
  const double f=P2.at<double>(i,i);
  const double dnear=offset-f*out.signed_baseline_mm/near_mm;
  const double dfar=offset-f*out.signed_baseline_mm/far_mm;
  // A one-pixel allowance covers integer matching and finite image sampling.
  out.disparity_min_px=std::floor(std::min(dnear,dfar)-1);
  out.disparity_max_px=std::ceil(std::max(dnear,dfar)+1);
  if (!std::isfinite(out.signed_baseline_mm) || !std::isfinite(dnear) ||
      !std::isfinite(dfar) || !std::isfinite(out.disparity_min_px) ||
      !std::isfinite(out.disparity_max_px) ||
      out.disparity_max_px-out.disparity_min_px>double(a.image_size.width+a.image_size.height))
    return {Status::invalid_input,"invalid or excessive disparity search range"};
  try {
    cv::initUndistortRectifyMap(cv::Mat(a.intrinsic), a.distortion, R1, P1,
                                a.image_size, CV_32FC1, out.map1x, out.map1y);
    cv::initUndistortRectifyMap(cv::Mat(b.intrinsic), b.distortion, R2, P2,
                                b.image_size, CV_32FC1, out.map2x, out.map2y);
  } catch (const cv::Exception& e) { return {Status::invalid_input,e.what()}; }
  if (!cv::checkRange(out.map1x) || !cv::checkRange(out.map1y) ||
      !cv::checkRange(out.map2x) || !cv::checkRange(out.map2y))
    return {Status::invalid_input,"nonfinite rectification map"};
  return {};
}

Result triangulate_object(const Camera& a, const Camera& b,
                          cv::Point2d p1, cv::Point2d p2, cv::Vec3d& object_mm) {
  if (!valid(a) || !valid(b) || a.image_size != b.image_size ||
      !std::isfinite(p1.x) || !std::isfinite(p1.y) || !std::isfinite(p2.x) || !std::isfinite(p2.y) ||
      p1.x<0 || p1.y<0 || p2.x<0 || p2.y<0 ||
      p1.x>=a.image_size.width || p1.y>=a.image_size.height ||
      p2.x>=b.image_size.width || p2.y>=b.image_size.height)
    return {Status::invalid_input, "invalid camera or pixel"};
  const cv::Matx33d r=b.pose.rotation*a.pose.rotation.t();
  const cv::Vec3d t=b.pose.translation_mm-r*a.pose.translation_mm;
  if (cv::norm(t)<1e-3) return {Status::degenerate_pose, "baseline below 0.001 mm"};
  try {
  std::vector<cv::Point2d> u1,u2;
  cv::undistortPoints(std::vector<cv::Point2d>{p1},u1,cv::Mat(a.intrinsic),a.distortion);
  cv::undistortPoints(std::vector<cv::Point2d>{p2},u2,cv::Mat(b.intrinsic),b.distortion);
  if (!std::isfinite(u1[0].x) || !std::isfinite(u1[0].y) ||
      !std::isfinite(u2[0].x) || !std::isfinite(u2[0].y))
    return {Status::invalid_sample,"nonfinite undistorted ray"};
  cv::Vec3d ray1(u1[0].x,u1[0].y,1), ray2(u2[0].x,u2[0].y,1);
  ray1*=1/cv::norm(ray1); ray2=r.t()*ray2; ray2*=1/cv::norm(ray2);
  if (cv::norm(ray1.cross(ray2))<std::sin(0.1*CV_PI/180))
    return {Status::invalid_sample,"parallax below 0.1 degree"};
  cv::Matx34d P1(1,0,0,0, 0,1,0,0, 0,0,1,0);
  cv::Matx34d P2(r(0,0),r(0,1),r(0,2),t[0], r(1,0),r(1,1),r(1,2),t[1], r(2,0),r(2,1),r(2,2),t[2]);
  cv::Mat h;
  cv::triangulatePoints(cv::Mat(P1),cv::Mat(P2),u1,u2,h);
  const double w=h.at<double>(3,0);
  if (!std::isfinite(w) || std::abs(w)<1e-12) return {Status::invalid_sample,"rays do not intersect at finite depth"};
  const cv::Vec3d x(h.at<double>(0,0)/w,h.at<double>(1,0)/w,h.at<double>(2,0)/w);
  if (!finite(x) || x[2]<=0 || (r*x+t)[2]<=0) return {Status::invalid_sample,"triangulated point is behind a camera"};
  std::vector<cv::Point2d> reproj1,reproj2;
  cv::projectPoints(std::vector<cv::Point3d>{cv::Point3d(x)},cv::Vec3d(0,0,0),cv::Vec3d(0,0,0),
                    cv::Mat(a.intrinsic),a.distortion,reproj1);
  cv::projectPoints(std::vector<cv::Point3d>{cv::Point3d(r*x+t)},cv::Vec3d(0,0,0),cv::Vec3d(0,0,0),
                    cv::Mat(b.intrinsic),b.distortion,reproj2);
  if (!std::isfinite(reproj1[0].x) || !std::isfinite(reproj1[0].y) ||
      !std::isfinite(reproj2[0].x) || !std::isfinite(reproj2[0].y) ||
      cv::norm(reproj1[0]-p1)>2 || cv::norm(reproj2[0]-p2)>2)
    return {Status::invalid_sample,"reprojection residual above 2 pixels"};
  object_mm=a.pose.rotation.t()*(x-a.pose.translation_mm);
  if (!finite(object_mm))
    return {Status::invalid_sample,"nonfinite object-frame point"};
  return {};
  } catch (const cv::Exception& e) { return {Status::invalid_input,e.what()}; }
}

namespace {
struct Support { int views=0; bool conflict=false; };
Support check_support(const cv::Vec3d& object_mm,
                      const std::vector<DepthView>& views,double tolerance_mm) {
  Support result;
  std::vector<cv::Vec3d> supported_centers;
  for (const auto& v:views) {
    const auto x=camera_point(v.camera,object_mm);
    if (!finite(x) || x[2]<=0) continue;
    std::vector<cv::Point2d> px;
    cv::projectPoints(std::vector<cv::Point3d>{cv::Point3d(x)},cv::Vec3d(0,0,0),
                      cv::Vec3d(0,0,0),cv::Mat(v.camera.intrinsic),v.camera.distortion,px);
    if (!std::isfinite(px[0].x) || !std::isfinite(px[0].y) ||
        px[0].x<-1 || px[0].y<-1 ||
        px[0].x>v.depth_mm.cols || px[0].y>v.depth_mm.rows) continue;
    const int col=cvRound(px[0].x), row=cvRound(px[0].y);
    if (row<0 || col<0 || row>=v.depth_mm.rows || col>=v.depth_mm.cols ||
        v.object_mask.at<unsigned char>(row,col)==0) continue;
    const float observed=v.depth_mm.at<float>(row,col);
    if (!std::isfinite(observed) || observed<=0) continue;
    if (double(observed)<x[2]-tolerance_mm) continue;
    if (double(observed)>x[2]+tolerance_mm) { result.conflict=true; return result; }
    const cv::Vec3d center=-v.camera.pose.rotation.t()*v.camera.pose.translation_mm;
    bool distinct=true;
    for (const auto& prior:supported_centers)
      if (cv::norm(prior-center)<1e-3) distinct=false;
    if (distinct) { supported_centers.push_back(center); ++result.views; }
  }
  return result;
}
}  // namespace

Result fuse_tracks(const std::vector<Candidate>& candidates,
                   const std::vector<DepthView>& views, double tolerance_mm,
                   std::vector<FusedPoint>& points, FusionStats& stats) {
  points.clear(); stats={}; stats.input_candidates=candidates.size();
  if (views.size()<2 || views.size()>64 || candidates.size()>1000000 ||
      !std::isfinite(tolerance_mm) || tolerance_mm<=0)
    return {Status::invalid_input,"invalid fusion resource count or tolerance"};
  int64_t total_pixels=0;
  for (const auto& v:views) {
    total_pixels+=int64_t(v.camera.image_size.width)*v.camera.image_size.height;
    if (!valid(v.camera) || v.depth_mm.type()!=CV_32FC1 ||
        v.object_mask.type()!=CV_8UC1 || v.depth_mm.size()!=v.camera.image_size ||
        v.object_mask.size()!=v.camera.image_size)
      return {Status::invalid_input,"invalid depth view or object mask"};
  }
  if (total_pixels>128000000) return {Status::invalid_input,"aggregate pixel cap exceeded"};
  std::map<int,std::vector<std::pair<Candidate,int>>> groups;
  try {
  for (const auto& c:candidates) {
    if (c.track_id<0 || !finite(c.object_mm) || cv::norm(c.object_mm)>1e9 ||
        !std::isfinite(c.confidence) || c.confidence<=0 || c.confidence>1e6) {
      ++stats.rejected_candidates; continue;
    }
    const Support support=check_support(c.object_mm,views,tolerance_mm);
    if (support.conflict || support.views<2) { ++stats.rejected_candidates; continue; }
    groups[c.track_id].push_back({c,support.views}); ++stats.accepted_candidates;
  }
  for (auto& [id, members]:groups) {
    // Keep the strongest mutually consistent set; a separate surface cannot
    // pull the centroid through a thin wall or discontinuity.
    auto seed=std::max_element(members.begin(),members.end(),[](const auto& a,const auto& b){return a.first.confidence<b.first.confidence;});
    cv::Vec3d sum(0,0,0); double total=0; int support=0;
    for (const auto& [c,n]:members) if (cv::norm(c.object_mm-seed->first.object_mm)<=tolerance_mm) {
      sum+=c.object_mm*c.confidence; total+=c.confidence; support=std::max(support,n);
    } else {
      ++stats.discarded_by_cluster;
    }
    if (total>0) {
      const cv::Vec3d center=sum/total;
      const Support centroid_support=check_support(center,views,tolerance_mm);
      if (!centroid_support.conflict && centroid_support.views>=2)
        points.push_back({id,center,total,centroid_support.views});
      else ++stats.rejected_centroids;
    }
  }
  stats.output_points=points.size();
  return {};
  } catch (const cv::Exception& e) {
    points.clear(); return {Status::invalid_input,e.what()};
  }
}
}  // namespace crispmesh::depth
