#include "depth_geometry.h"
#include <opencv2/calib3d.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <sstream>
#include <stdexcept>
#include <vector>

using namespace crispmesh::depth;
namespace {
// Frozen before the first measured run. These are experiment settings, not
// parameters derived from the renderer's depth or expected error.
constexpr int kBlock=5, kStep=3, kErode=3;
constexpr double kNear=400, kFar=560, kLrPx=1.5, kCrossMm=8, kFuseMm=8;
constexpr double kMissingErrorMm=100;
void check(bool ok,const char* why) { if (!ok) throw std::runtime_error(why); }
cv::Matx33d yaw(double a) {
  const double s=std::sin(a), c=std::cos(a);
  return {c,0,s, 0,1,0, -s,0,c};
}
Camera camera(double center_x) {
  Camera c;
  c.image_size={640,480}; c.intrinsic={800,0,320, 0,800,240, 0,0,1};
  c.pose.rotation=yaw(0.025);
  c.pose.translation_mm=-(c.pose.rotation*cv::Vec3d(center_x,0,0));
  return c;
}
struct Hit { bool object=false, front=false; cv::Vec3d point; };
Hit hit(const Camera& c,double u,double v) {
  const cv::Vec3d origin=-c.pose.rotation.t()*c.pose.translation_mm;
  const cv::Vec3d ray=c.pose.rotation.t()*cv::Vec3d((u-320)/800,(v-240)/800,1);
  auto at=[&](double z) { return origin+ray*((z-origin[2])/ray[2]); };
  cv::Vec3d p=at(450);
  const bool front=std::abs(p[0])<26 && std::abs(p[1])<30;
  if (!front) p=at(500);
  return {std::abs(p[0])<=105 && std::abs(p[1])<=78,front,p};
}
void render(const Camera& c,cv::Mat& im,cv::Mat& mask) {
  im=cv::Mat(c.image_size,CV_8UC1); mask=cv::Mat::zeros(c.image_size,CV_8UC1);
  for (int y=0;y<im.rows;++y) for (int x=0;x<im.cols;++x) {
    // Exterior texture is intentionally unrelated to the object texture.
    im.at<uchar>(y,x)=uchar(25+(x*37+y*73)%130);
    const Hit h=hit(c,x,y);
    if (!h.object) continue;
    mask.at<uchar>(y,x)=255;
    const auto& q=h.point;
    const double signal=128+34*std::sin(0.43*q[0]+0.11*q[1])
      +28*std::sin(0.09*q[0]-0.59*q[1])
      +21*std::sin(0.71*q[0]+0.67*q[1])
      +18*std::sin(0.23*q[0]+0.31*q[1]);
    im.at<uchar>(y,x)=cv::saturate_cast<uchar>(signal);
  }
}
struct Pair {
  Camera first,second;
  RectifiedPair g;
  cv::Mat d,valid,first_image,second_image,first_mask,second_mask;
  int left_right_rejected=0;
};
cv::Ptr<cv::StereoSGBM> matcher(int min_d,int count) {
  return cv::StereoSGBM::create(min_d,count,kBlock,8*kBlock*kBlock,
    32*kBlock*kBlock,1,0,0,0,0,cv::StereoSGBM::MODE_SGBM);
}
Pair match(const Camera& a,const Camera& b,const cv::Mat& ia,const cv::Mat& ib,
           const cv::Mat& ma,const cv::Mat& mb) {
  Pair p; p.first=a; p.second=b;
  check(bool(rectify(a,b,kNear,kFar,p.g)),"rectify");
  check(p.g.axis==Axis::horizontal,"fixture axis");
  cv::remap(ia,p.first_image,p.g.map1x,p.g.map1y,cv::INTER_LINEAR);
  cv::remap(ib,p.second_image,p.g.map2x,p.g.map2y,cv::INTER_LINEAR);
  cv::remap(ma,p.first_mask,p.g.map1x,p.g.map1y,cv::INTER_NEAREST);
  cv::remap(mb,p.second_mask,p.g.map2x,p.g.map2y,cv::INTER_NEAREST);
  cv::erode(p.first_mask,p.first_mask,cv::Mat(),cv::Point(-1,-1),kErode);
  cv::erode(p.second_mask,p.second_mask,cv::Mat(),cv::Point(-1,-1),kErode);
  // Exclude exterior pixels from cost evaluation as far as SGBM permits.
  p.first_image.setTo(0,p.first_mask==0);
  p.second_image.setTo(0,p.second_mask==0);
  const int min_d=std::max(0,int(std::floor(p.g.disparity_min_px)));
  const int max_d=int(std::ceil(p.g.disparity_max_px));
  const int count=16*((max_d-min_d+16)/16);
  const int reverse_min=-max_d-1;
  const int reverse_count=16*((-min_d-reverse_min+16)/16);
  cv::Mat reverse;
  matcher(min_d,count)->compute(p.first_image,p.second_image,p.d);
  matcher(reverse_min,reverse_count)->compute(p.second_image,p.first_image,reverse);
  p.valid=cv::Mat::zeros(a.image_size,CV_8UC1);
  for (int y=4;y<p.d.rows-4;++y) for (int x=min_d+count+4;x<p.d.cols-4;++x) {
    if (!p.first_mask.at<uchar>(y,x)) continue;
    const double d=p.d.at<short>(y,x)/16.0;
    const int xr=cvRound(x-d);
    if (xr<4 || xr>=p.d.cols-4 || !p.second_mask.at<uchar>(y,xr) ||
        d<p.g.disparity_min_px || d>p.g.disparity_max_px) continue;
    const double reverse_d=reverse.at<short>(y,xr)/16.0;
    if (reverse_d<reverse_min || reverse_d>-min_d ||
        std::abs(d+reverse_d)>kLrPx) {
      ++p.left_right_rejected; continue;
    }
    p.valid.at<uchar>(y,x)=255;
  }
  return p;
}
cv::Vec3d object_point(const Pair& p,int x,int y) {
  const double d=p.d.at<short>(y,x)/16.0;
  const auto& P=p.g.projection1;
  const double z=P(0,0)*(-p.g.signed_baseline_mm)/
    (d-(P(0,2)-p.g.projection2(0,2)));
  const cv::Vec3d rect((x-P(0,2))*z/P(0,0),(y-P(1,2))*z/P(1,1),z);
  const cv::Vec3d camera=p.g.rectification1.t()*rect;
  return p.first.pose.rotation.t()*(camera-p.first.pose.translation_mm);
}
DepthView depth_view(const Pair& p) {
  DepthView v; v.camera=p.first;
  v.depth_mm=cv::Mat::zeros(p.first.image_size,CV_32FC1);
  v.object_mask=cv::Mat::zeros(p.first.image_size,CV_8UC1);
  for (int y=0;y<p.d.rows;++y) for (int x=0;x<p.d.cols;++x) {
    if (!p.valid.at<uchar>(y,x)) continue;
    const cv::Vec3d q=object_point(p,x,y);
    const cv::Vec3d cam=p.first.pose.rotation*q+p.first.pose.translation_mm;
    if (!std::isfinite(cam[2]) || cam[2]<=0) continue;
    const int u=cvRound(p.g.map1x.at<float>(y,x));
    const int w=cvRound(p.g.map1y.at<float>(y,x));
    if (u<0 || w<0 || u>=v.depth_mm.cols || w>=v.depth_mm.rows) continue;
    if (v.depth_mm.at<float>(w,u)==0 || cam[2]<v.depth_mm.at<float>(w,u))
      v.depth_mm.at<float>(w,u)=float(cam[2]);
    v.object_mask.at<uchar>(w,u)=255;
  }
  return v;
}
double percentile(std::vector<double> v,double f) {
  if (v.empty()) return -1;
  std::sort(v.begin(),v.end());
  return v[std::min(v.size()-1,size_t(f*(v.size()-1)))];
}
double mean(const std::vector<double>& v) {
  if (v.empty()) return -1;
  double sum=0; for (double x:v) sum+=x;
  return sum/v.size();
}
struct Score {
  size_t population=0,raw=0,accepted=0,raw_bad2=0,accepted_bad2=0;
  std::vector<double> raw_errors,accepted_errors,raw_fixed,accepted_fixed;
  void add(bool has_raw,double raw_err,bool has_accepted,double accepted_err) {
    ++population;
    if (has_raw) { ++raw; raw_errors.push_back(raw_err); }
    if (has_accepted) { ++accepted; accepted_errors.push_back(accepted_err); }
    const double r=has_raw ? raw_err:kMissingErrorMm;
    const double a=has_accepted ? accepted_err:kMissingErrorMm;
    raw_fixed.push_back(r); accepted_fixed.push_back(a);
    raw_bad2+=r>2; accepted_bad2+=a>2;
  }
};
void save(const Pair& ac,const DepthView& av,const DepthView& bv,
          const cv::Mat& accepted,const std::string& report,
          const std::filesystem::path& base) {
  check(std::filesystem::space(base).available>uintmax_t(10)*1024*1024*1024+20*1024*1024,
        "10 GiB artifact reserve");
  const auto stamp=std::chrono::duration_cast<std::chrono::microseconds>(
    std::chrono::system_clock::now().time_since_epoch()).count();
  std::filesystem::path out; bool created=false;
  for (int i=0;i<100;++i) {
    out=base/("run-"+std::to_string(stamp)+"-"+std::to_string(i));
    if (std::filesystem::create_directory(out)) { created=true; break; }
  }
  check(created,"fresh artifact directory");
  check(cv::imwrite((out/"masked-reference.png").string(),ac.first_image) &&
        cv::imwrite((out/"masked-third.png").string(),ac.second_image) &&
        cv::imwrite((out/"candidate-valid.png").string(),ac.valid) &&
        cv::imwrite((out/"fused-accepted.png").string(),accepted) &&
        cv::imwrite((out/"support-a.png").string(),av.object_mask) &&
        cv::imwrite((out/"support-b.png").string(),bv.object_mask),"save PNGs");
  std::ofstream file(out/"report.json"); file<<report<<'\n';
  check(file.good(),"save report");
  std::cout<<"artifactDirectory="<<out.string()<<'\n';
}
} // namespace

int main(int argc,char** argv) {
  try {
    const auto started=std::chrono::steady_clock::now();
    const Camera a=camera(0),b=camera(40),c=camera(70);
    cv::Mat ia,ib,ic,ma,mb,mc;
    render(a,ia,ma); render(b,ib,mb); render(c,ic,mc);
    const Pair ac=match(a,c,ia,ic,ma,mc);
    const Pair ab=match(a,b,ia,ib,ma,mb);
    const Pair bc=match(b,c,ib,ic,mb,mc);
    const cv::Vec3d known(12,9,450);
    for (const Pair* p:{&ac,&bc}) {
      const Camera& first=p->first;
      const cv::Vec3d known_camera=first.pose.rotation*known+first.pose.translation_mm;
      const cv::Vec3d known_rect=p->g.rectification1*known_camera;
      check(cv::norm(first.pose.rotation.t()*(p->g.rectification1.t()*known_rect-
            first.pose.translation_mm)-known)<1e-9,"rectified-to-object transform");
    }
    check(cv::norm(ac.g.map1x-ab.g.map1x,cv::NORM_INF)<1e-4,
          "reference rectification grids differ");
    // Replacing every outside-mask source pixel must leave masked matcher
    // inputs and disparity unchanged, including the global SGBM paths.
    cv::Mat ia2=ia.clone(),ib2=ib.clone(),ic2=ic.clone();
    ia2.setTo(243,ma==0); ib2.setTo(7,mb==0); ic2.setTo(211,mc==0);
    const Pair mutated=match(a,c,ia2,ic2,ma,mc);
    const Pair mutated_ab=match(a,b,ia2,ib2,ma,mb);
    const Pair mutated_bc=match(b,c,ib2,ic2,mb,mc);
    check(cv::norm(ac.first_image-mutated.first_image,cv::NORM_INF)==0 &&
          cv::norm(ac.second_image-mutated.second_image,cv::NORM_INF)==0 &&
          cv::norm(ac.d-mutated.d,cv::NORM_INF)==0 &&
          cv::norm(ac.valid-mutated.valid,cv::NORM_INF)==0,
          "outside-mask mutation changed matching");
    check(cv::norm(ab.d-mutated_ab.d,cv::NORM_INF)==0 &&
          cv::norm(ab.valid-mutated_ab.valid,cv::NORM_INF)==0 &&
          cv::norm(bc.d-mutated_bc.d,cv::NORM_INF)==0 &&
          cv::norm(bc.valid-mutated_bc.valid,cv::NORM_INF)==0,
          "outside-mask mutation changed support matching");
    const DepthView av=depth_view(ab),bv=depth_view(bc);
    std::vector<Candidate> candidates;
    std::vector<Candidate> dual_candidates;
    struct Eval { bool raw=false,edge=false,front=false; int x=0,y=0; double expected=0,raw_error=0; };
    std::vector<Eval> eval;
    size_t lr_valid=0,cross_valid=0,near_count=0,far_count=0;
    // Fixed full-frame foreground grid includes the disparity search strip and
    // eroded edges, even when the matcher can never return an estimate there.
    cv::Mat original_rectified_mask;
    cv::remap(ma,original_rectified_mask,ac.g.map1x,ac.g.map1y,cv::INTER_NEAREST);
    for (int y=0;y<ac.d.rows;y+=kStep) for (int x=0;x<ac.d.cols;x+=kStep) {
      if (!original_rectified_mask.at<uchar>(y,x)) continue;
      const Hit truth=hit(a,ac.g.map1x.at<float>(y,x),ac.g.map1y.at<float>(y,x));
      if (!truth.object) continue;
      Eval e; e.expected=truth.point[2]; e.front=truth.front; e.x=x; e.y=y;
      const cv::Vec3d near_at=truth.front ? truth.point :
        cv::Vec3d(truth.point[0]*450/truth.point[2],truth.point[1]*450/truth.point[2],450);
      e.edge=std::abs(std::abs(near_at[0])-26)<4 ||
        (std::abs(near_at[0])<30 && std::abs(std::abs(near_at[1])-30)<4);
      if (e.front) ++near_count; else ++far_count;
      if (ac.valid.at<uchar>(y,x)) {
        ++lr_valid;
        const cv::Vec3d q=object_point(ac,x,y);
        if (std::isfinite(q[2]) && q[2]>0) {
          e.raw=true; e.raw_error=std::abs(q[2]-e.expected);
          // Only image measurements determine candidate position and confidence.
          candidates.push_back({q,1.0,int(eval.size())});
          dual_candidates.push_back({q,1.0,int(eval.size())});
        }
      }
      if (ab.valid.at<uchar>(y,x)) {
        const cv::Vec3d q=object_point(ab,x,y);
        if (std::isfinite(q[2]) && q[2]>0)
          dual_candidates.push_back({q,1.0,int(eval.size())});
      }
      eval.push_back(e);
    }
    check(eval.size()<1000000,"candidate resource cap");
    // Direct cross-view agreement is also counted independently of fusion.
    for (const Candidate& candidate:candidates) {
      auto agrees=[&](const DepthView& v) {
        const cv::Vec3d cam=v.camera.pose.rotation*candidate.object_mm+v.camera.pose.translation_mm;
        const int x=cvRound(v.camera.intrinsic(0,0)*cam[0]/cam[2]+v.camera.intrinsic(0,2));
        const int y=cvRound(v.camera.intrinsic(1,1)*cam[1]/cam[2]+v.camera.intrinsic(1,2));
        return x>=0 && y>=0 && x<v.depth_mm.cols && y<v.depth_mm.rows &&
          v.object_mask.at<uchar>(y,x) &&
          std::abs(v.depth_mm.at<float>(y,x)-cam[2])<=kCrossMm;
      };
      cross_valid+=agrees(av) && agrees(bv);
    }
    std::vector<FusedPoint> fused; FusionStats stats;
    check(bool(fuse_tracks(candidates,{av,bv},kFuseMm,fused,stats)),"fuse_tracks");
    std::vector<FusedPoint> dual_fused; FusionStats dual_stats;
    check(bool(fuse_tracks(dual_candidates,{av,bv},kFuseMm,dual_fused,dual_stats)),
          "two-candidate fuse_tracks");
    cv::Mat accepted=cv::Mat::zeros(ac.d.size(),CV_8UC1);
    std::vector<double> fused_error(eval.size(),-1);
    std::vector<double> dual_error(eval.size(),-1);
    for (const auto& p:fused) {
      check(p.track_id>=0 && size_t(p.track_id)<eval.size(),"track ID range");
      fused_error[p.track_id]=std::abs(p.object_mm[2]-eval[p.track_id].expected);
      accepted.at<uchar>(eval[p.track_id].y,eval[p.track_id].x)=255;
    }
    for (const auto& p:dual_fused) {
      check(p.track_id>=0 && size_t(p.track_id)<eval.size(),"dual track ID range");
      dual_error[p.track_id]=std::abs(p.object_mm[2]-eval[p.track_id].expected);
    }
    Score all,edge,front,back;
    Score dual_all,dual_edge,dual_front,dual_back;
    for (size_t i=0;i<eval.size();++i) {
      const auto& e=eval[i]; const bool ok=fused_error[i]>=0;
      const bool dual_ok=dual_error[i]>=0;
      all.add(e.raw,e.raw_error,ok,fused_error[i]);
      dual_all.add(e.raw,e.raw_error,dual_ok,dual_error[i]);
      if (e.edge) edge.add(e.raw,e.raw_error,ok,fused_error[i]);
      if (e.edge) dual_edge.add(e.raw,e.raw_error,dual_ok,dual_error[i]);
      if (e.front) front.add(e.raw,e.raw_error,ok,fused_error[i]);
      else back.add(e.raw,e.raw_error,ok,fused_error[i]);
      if (e.front) dual_front.add(e.raw,e.raw_error,dual_ok,dual_error[i]);
      else dual_back.add(e.raw,e.raw_error,dual_ok,dual_error[i]);
    }
    const auto elapsed=std::chrono::duration<double,std::milli>(
      std::chrono::steady_clock::now()-started).count();
    auto fields=[](const char* name,const Score& s) {
      std::ostringstream o;
      o<<"\""<<name<<"Population\":"<<s.population
       <<",\""<<name<<"Raw\":"<<s.raw
       <<",\""<<name<<"Accepted\":"<<s.accepted
       <<",\""<<name<<"RawCompleteness\":"<<double(s.raw)/s.population
       <<",\""<<name<<"AcceptedCompleteness\":"<<double(s.accepted)/s.population
       <<",\""<<name<<"RawBad2Fixed\":"<<s.raw_bad2
       <<",\""<<name<<"AcceptedBad2Fixed\":"<<s.accepted_bad2
       <<",\""<<name<<"RawMeanFixedMm\":"<<mean(s.raw_fixed)
       <<",\""<<name<<"AcceptedMeanFixedMm\":"<<mean(s.accepted_fixed)
       <<",\""<<name<<"RawP95ObservedMm\":"<<percentile(s.raw_errors,0.95)
       <<",\""<<name<<"AcceptedP95ObservedMm\":"<<percentile(s.accepted_errors,0.95)
       <<",\""<<name<<"RawP95FixedMm\":"<<percentile(s.raw_fixed,0.95)
       <<",\""<<name<<"AcceptedP95FixedMm\":"<<percentile(s.accepted_fixed,0.95);
      return o.str();
    };
    std::ostringstream report;
    const bool control_whole_better=all.accepted_bad2<all.raw_bad2;
    const bool control_edge_better=edge.accepted_bad2<edge.raw_bad2;
    const bool dual_whole_better=dual_all.accepted_bad2<dual_all.raw_bad2;
    const bool dual_edge_better=dual_edge.accepted_bad2<dual_edge.raw_bad2;
    report<<"{\"ok\":true,\"kind\":\"image_to_fuse_tracks\",\"views\":3,"
      <<"\"productionAccepted\":false"
      <<",\"twoCandidateQualityGatePassed\":"
      <<(dual_whole_better && dual_edge_better ? "true":"false")
      <<",\"controlWholeBad2Improved\":"<<(control_whole_better ? "true":"false")
      <<",\"controlBoundaryBad2Improved\":"<<(control_edge_better ? "true":"false")
      <<",\"twoCandidateWholeBad2Improved\":"<<(dual_whole_better ? "true":"false")
      <<",\"twoCandidateBoundaryBad2Improved\":"<<(dual_edge_better ? "true":"false")<<','
      <<"\"lrValid\":"<<lr_valid<<",\"crossViewValid\":"<<cross_valid
      <<",\"nearPopulation\":"<<near_count<<",\"farPopulation\":"<<far_count
      <<",\"leftRightRejectedAC\":"<<ac.left_right_rejected
      <<",\"leftRightRejectedAB\":"<<ab.left_right_rejected
      <<",\"leftRightRejectedBC\":"<<bc.left_right_rejected
      <<",\"fusionRejected\":"<<stats.rejected_candidates
      <<",\"fusionAccepted\":"<<stats.accepted_candidates
      <<",\"fusionOutput\":"<<stats.output_points<<','
      <<"\"twoCandidateInput\":"<<dual_candidates.size()
      <<",\"twoCandidateAccepted\":"<<dual_stats.accepted_candidates
      <<",\"twoCandidateOutput\":"<<dual_stats.output_points<<','
      <<fields("all",all)<<','<<fields("edge",edge)<<','
      <<fields("front",front)<<','<<fields("back",back)
      <<','<<fields("dualAll",dual_all)<<','<<fields("dualEdge",dual_edge)
      <<','<<fields("dualFront",dual_front)<<','<<fields("dualBack",dual_back)
      <<",\"elapsedMs\":"<<elapsed<<"}";
    std::cout<<report.str()<<'\n';
    // Frozen acceptance: enough coverage on each surface; fixed-population
    // bad2 includes every missing output with the predeclared 100 mm penalty.
    check(all.population>1000 && front.population>100 && back.population>500,
          "insufficient fixed foreground population");
    check(all.raw>200 && all.accepted>80 && all.accepted<=all.raw,
          "insufficient image-derived fusion coverage");
    check(all.accepted_errors.size()==stats.output_points,"fused output accounting");
    check(dual_all.accepted_errors.size()==dual_stats.output_points,
          "two-candidate output accounting");
    if (argc>1) {
      const std::filesystem::path base(argv[1]);
      std::filesystem::create_directories(base);
      save(ac,av,bv,accepted,report.str(),base);
    }
    return EXIT_SUCCESS;
  } catch (const std::exception& e) {
    std::cerr<<"image-derived fusion test failed: "<<e.what()<<'\n';
    return EXIT_FAILURE;
  }
}
