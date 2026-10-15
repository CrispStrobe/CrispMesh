#include "depth_geometry.h"
#include <opencv2/calib3d.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/imgcodecs.hpp>
#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <stdexcept>

using namespace crispmesh::depth;
namespace {
void check(bool ok,const char* msg) { if (!ok) throw std::runtime_error(msg); }
Camera camera(double center_x) {
  Camera c;
  c.image_size={640,480};
  c.intrinsic=cv::Matx33d(800,0,320, 0,800,240, 0,0,1);
  c.pose.translation_mm={-center_x,0,0};
  return c;
}
// Independent pinhole ray/analytic two-plane renderer. The foreground patch
// occludes the farther panel; world-space texture is stable between views.
void render(const Camera& c, cv::Mat& image, cv::Mat& mask) {
  image=cv::Mat(c.image_size,CV_8UC1);
  mask=cv::Mat(c.image_size,CV_8UC1,cv::Scalar(0));
  const double center=-c.pose.translation_mm[0];
  for (int y=0;y<image.rows;++y) for (int x=0;x<image.cols;++x) {
    image.at<unsigned char>(y,x)=static_cast<unsigned char>(30+((x*37+y*73+int(center)*19)%120));
    auto sample=[&](double z) {
      return cv::Point2d(center+(x-320)*z/800.0,(y-240)*z/800.0);
    };
    auto q=sample(450); bool near=std::abs(q.x)<26 && std::abs(q.y)<30;
    double z=near ? 450 : 500;
    if (!near) q=sample(z);
    if (std::abs(q.x)>105 || std::abs(q.y)>78) continue;
    mask.at<unsigned char>(y,x)=255;
    const double signal=128+34*std::sin(0.43*q.x+0.11*q.y)
                           +28*std::sin(0.09*q.x-0.59*q.y)
                           +21*std::sin(0.71*q.x+0.67*q.y)
                           +18*std::sin(0.23*q.x+0.31*q.y);
    image.at<unsigned char>(y,x)=cv::saturate_cast<unsigned char>(signal);
  }
}
struct Match {
  cv::Mat disparity, mask, rectified_first, rectified_second;
  RectifiedPair geometry;
  Camera second;
};
Match match(const Camera& first, const Camera& second,
            const cv::Mat& first_image, const cv::Mat& second_image,
            const cv::Mat& first_mask, const cv::Mat& second_mask) {
  Match m; m.second=second;
  check(bool(rectify(first,second,400,560,m.geometry)),"rectification");
  check(m.geometry.axis==Axis::horizontal,"image fixture axis");
  cv::Mat a,b,ma,mb;
  cv::remap(first_image,a,m.geometry.map1x,m.geometry.map1y,cv::INTER_LINEAR);
  cv::remap(second_image,b,m.geometry.map2x,m.geometry.map2y,cv::INTER_LINEAR);
  m.rectified_first=a; m.rectified_second=b;
  cv::remap(first_mask,ma,m.geometry.map1x,m.geometry.map1y,cv::INTER_NEAREST);
  cv::remap(second_mask,mb,m.geometry.map2x,m.geometry.map2y,cv::INTER_NEAREST);
  cv::erode(ma,ma,cv::Mat(),cv::Point(-1,-1),3);
  cv::erode(mb,mb,cv::Mat(),cv::Point(-1,-1),3);
  const int minimum=std::max(0,int(std::floor(m.geometry.disparity_min_px)));
  const int range=int(std::ceil(m.geometry.disparity_max_px))-minimum+1;
  const int count=16*((range+15)/16);
  auto sgbm=cv::StereoSGBM::create(minimum,count,5,8*5*5,32*5*5,1,0,0,0,0,
                                   cv::StereoSGBM::MODE_SGBM);
  sgbm->compute(a,b,m.disparity);
  m.mask=cv::Mat(ma.size(),CV_8UC1,cv::Scalar(0));
  for (int y=4;y<ma.rows-4;++y) for (int x=minimum+count+4;x<ma.cols-4;++x) {
    const double d=m.disparity.at<short>(y,x)/16.0;
    const int x2=cvRound(x-d);
    if (ma.at<unsigned char>(y,x) && x2>=4 && x2<mb.cols-4 &&
        mb.at<unsigned char>(y,x2) && d>=m.geometry.disparity_min_px &&
        d<=m.geometry.disparity_max_px) m.mask.at<unsigned char>(y,x)=255;
  }
  return m;
}
cv::Vec3d reconstruct(const Match& m, int x, int y) {
  const double d=m.disparity.at<short>(y,x)/16.0;
  const auto& P=m.geometry.projection1;
  const double z=P(0,0)*(-m.geometry.signed_baseline_mm)/
                 (d-(P(0,2)-m.geometry.projection2(0,2)));
  const cv::Vec3d rect((x-P(0,2))*z/P(0,0),
                       (y-P(1,2))*z/P(1,1),z);
  return m.geometry.rectification1.t()*rect;
}
double percentile(std::vector<double> errors,double fraction) {
  if (errors.empty()) return -1;
  std::sort(errors.begin(),errors.end());
  return errors[std::min(errors.size()-1,size_t(fraction*(errors.size()-1)))];
}
void save_disparity(const cv::Mat& disparity,const std::filesystem::path& path) {
  // Signed 1/16 px SGBM disparity is offset by 32768 in this 16-bit PNG.
  cv::Mat encoded(disparity.size(),CV_16UC1);
  for (int y=0;y<disparity.rows;++y) for (int x=0;x<disparity.cols;++x)
    encoded.at<uint16_t>(y,x)=uint16_t(int(disparity.at<int16_t>(y,x))+32768);
  check(cv::imwrite(path.string(),encoded),"save disparity");
}
}  // namespace

int main(int argc,char** argv) {
  try {
    const auto started=std::chrono::steady_clock::now();
    Camera a=camera(0),b=camera(40),c=camera(70);
    cv::Mat ia,ib,ic,ma,mb,mc;
    render(a,ia,ma); render(b,ib,mb); render(c,ic,mc);
    Match ab=match(a,b,ia,ib,ma,mb),ac=match(a,c,ia,ic,ma,mc);
    check(cv::norm(ab.geometry.rectification1-ac.geometry.rectification1)<1e-8 &&
          cv::norm(ab.geometry.projection1-ac.geometry.projection1)<1e-8 &&
          cv::norm(ab.geometry.map1x-ac.geometry.map1x,cv::NORM_INF)<1e-4,
          "pair reference grids differ");
    cv::Mat refmask;
    cv::remap(ma,refmask,ab.geometry.map1x,ab.geometry.map1y,cv::INTER_NEAREST);
    size_t requested=0,eligible=0,ab_valid=0,ac_valid=0,checked=0;
    size_t near_count=0,far_count=0,consistent=0,edge_eligible=0,edge_consistent=0;
    std::vector<double> errors,ab_all,ac_all,ab_common,ac_common,near_errors,far_errors,edge_errors;
    for (int y=20;y<460;y+=3) for (int x=180;x<460;x+=3) {
      ++requested;
      if (!refmask.at<unsigned char>(y,x)) continue;
      ++eligible;
      const auto near_q=cv::Point2d((x-320)*450.0/800,(y-240)*450.0/800);
      const bool front=std::abs(near_q.x)<26 && std::abs(near_q.y)<30;
      const bool edge=std::abs(std::abs(near_q.x)-26)<3 ||
                      (std::abs(near_q.x)<29 && std::abs(std::abs(near_q.y)-30)<3);
      if (edge) ++edge_eligible;
      const double expected=front ? 450 : 500;
      bool ab_ok=ab.mask.at<unsigned char>(y,x)!=0;
      bool ac_ok=ac.mask.at<unsigned char>(y,x)!=0;
      if (ab_ok) { ++ab_valid; ab_all.push_back(std::abs(reconstruct(ab,x,y)[2]-expected)); }
      if (ac_ok) { ++ac_valid; ac_all.push_back(std::abs(reconstruct(ac,x,y)[2]-expected)); }
      if (!ab_ok || !ac_ok) continue;
      ++checked;
      cv::Vec3d p1=reconstruct(ab,x,y),p2=reconstruct(ac,x,y);
      if (!std::isfinite(p1[2]) || !std::isfinite(p2[2]) ||
          cv::norm(p1-p2)>6) continue;
      ++consistent;
      const cv::Vec3d p=(p1+p2)*0.5; // two independently matched pairs
      ab_common.push_back(std::abs(p1[2]-expected));
      ac_common.push_back(std::abs(p2[2]-expected));
      if (front) ++near_count; else ++far_count;
      const double error=std::abs(p[2]-expected);
      errors.push_back(error);
      if (front) near_errors.push_back(error); else far_errors.push_back(error);
      if (edge) { ++edge_consistent; edge_errors.push_back(error); }
    }
    check(checked>100,"insufficient image-derived pair coverage");
    check(consistent>80 && consistent>checked/3,"insufficient cross-pair consistency");
    check(near_count>10 && far_count>50,"near/far surface coverage");
    const double median=percentile(errors,0.5),p95=percentile(errors,0.95);
    check(median<5 && p95<25,"image-derived metric depth error");
    const auto elapsed=std::chrono::duration<double,std::milli>(std::chrono::steady_clock::now()-started).count();
    std::ostringstream report;
    report<<"{\"ok\":true,\"kind\":\"synthetic_image_derived\",\"views\":3,"
             <<"\"requestedPixels\":"<<requested<<",\"eligiblePixels\":"<<eligible
             <<",\"abValidPixels\":"<<ab_valid<<",\"acValidPixels\":"<<ac_valid
             <<",\"checkedPixels\":"<<checked<<",\"consistentPixels\":"<<consistent
             <<",\"nearPixels\":"<<near_count<<",\"farPixels\":"<<far_count
             <<",\"edgeEligiblePixels\":"<<edge_eligible
             <<",\"edgeConsistentPixels\":"<<edge_consistent
             <<",\"medianDepthErrorMm\":"<<median<<",\"p95DepthErrorMm\":"<<p95
             <<",\"nearP95DepthErrorMm\":"<<percentile(near_errors,0.95)
             <<",\"farP95DepthErrorMm\":"<<percentile(far_errors,0.95)
             <<",\"edgeP95DepthErrorMm\":"<<percentile(edge_errors,0.95)
             <<",\"abP95AllMm\":"<<percentile(ab_all,0.95)
             <<",\"acP95AllMm\":"<<percentile(ac_all,0.95)
             <<",\"abP95ConsistentMm\":"<<percentile(ab_common,0.95)
             <<",\"acP95ConsistentMm\":"<<percentile(ac_common,0.95)
             <<",\"elapsedMs\":"<<elapsed<<"}";
    std::cout<<report.str()<<'\n';
    if (argc>1) {
      std::filesystem::path base(argv[1]);
      std::filesystem::create_directories(base);
      check(std::filesystem::space(base).available >= uintmax_t(10)*1024*1024*1024+20*1024*1024,
            "10 GiB free-space reserve for artifacts");
      const auto stamp=std::chrono::duration_cast<std::chrono::microseconds>(
          std::chrono::system_clock::now().time_since_epoch()).count();
      std::filesystem::path output;
      bool created=false;
      for (int n=0;n<100;++n) {
        output=base/("run-"+std::to_string(stamp)+"-"+std::to_string(n));
        if (std::filesystem::create_directory(output)) { created=true; break; }
      }
      check(created,"fresh artifact directory");
      check(cv::imwrite((output/"view-a.png").string(),ia) &&
            cv::imwrite((output/"view-b.png").string(),ib) &&
            cv::imwrite((output/"view-c.png").string(),ic) &&
            cv::imwrite((output/"mask-a.png").string(),ma) &&
            cv::imwrite((output/"mask-b.png").string(),mb) &&
            cv::imwrite((output/"mask-c.png").string(),mc) &&
            cv::imwrite((output/"rectified-a.png").string(),ab.rectified_first) &&
            cv::imwrite((output/"rectified-b.png").string(),ab.rectified_second) &&
            cv::imwrite((output/"rectified-c.png").string(),ac.rectified_second) &&
            cv::imwrite((output/"valid-ab.png").string(),ab.mask) &&
            cv::imwrite((output/"valid-ac.png").string(),ac.mask),"save fixture images");
      save_disparity(ab.disparity,output/"disparity-ab-signed16-offset32768.png");
      save_disparity(ac.disparity,output/"disparity-ac-signed16-offset32768.png");
      std::ofstream file(output/"report.json"); file<<report.str()<<'\n';
      check(file.good(),"save report");
      std::cout<<"artifactDirectory="<<output.string()<<'\n';
    }
    return EXIT_SUCCESS;
  } catch (const std::exception& e) {
    std::cerr<<"image-derived depth test failed: "<<e.what()<<'\n'; return EXIT_FAILURE;
  }
}
