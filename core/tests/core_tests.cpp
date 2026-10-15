#include "crispmesh/core.h"

#ifdef NDEBUG
#undef NDEBUG
#endif
#include <cassert>
#include <cstring>
#include <iostream>
#include <locale>
#include <string>
#include <vector>

namespace {
struct CommaDecimal : std::numpunct<char> {
  char do_decimal_point() const override { return ','; }
};
std::pair<crispmesh_status,std::string> run(const std::string &s, bool reconstruct=false) {
  auto fn = reconstruct ? crispmesh_reconstruct_json : crispmesh_validate_project_json;
  size_t needed=0;
  assert(fn(s.data(),s.size(),nullptr,0,&needed)==CRISPMESH_BUFFER_TOO_SMALL);
  std::vector<char> out(needed);
  auto status=fn(s.data(),s.size(),out.data(),out.size(),&needed);
  return {status,std::string(out.data())};
}
void check_invalid(const std::string &s, const std::string &fragment) {
  auto [status,json]=run(s);
  if (status != CRISPMESH_INVALID_PROJECT || json.find(fragment)==std::string::npos) {
    std::cerr << "Expected invalid project containing '" << fragment << "', got: " << json << '\n';
    std::abort();
  }
}
}

int main() {
  assert(crispmesh_abi_version()==1);
  const std::string empty=R"({"schemaVersion":1,"id":"scan-1","name":"Test","units":"mm","images":[],"stages":{}})";
  auto [status,valid]=run(empty);
  assert(status==CRISPMESH_OK && valid.find("\"imageCount\":0")!=std::string::npos);
  auto [reconstruction,unavailable]=run(empty,true);
  assert(reconstruction==CRISPMESH_UNAVAILABLE && unavailable.find("no mesh was produced")!=std::string::npos);
  check_invalid(R"({"schemaVersion":2,"id":"x","name":"x","units":"mm","images":[],"stages":{}})","schemaVersion");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[{"id":"a","path":"../secret.jpg"}],"stages":{}})","unsafe path segment");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[{"id":"a","path":"C:\\x.jpg"}],"stages":{}})","relative / separators");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[{"id":"a","path":"a.jpg"},{"id":"a","path":"b.jpg"}],"stages":{}})","unique");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[],"board":{"type":"aruco","markerSizeMm":20,"rotatesWithObject":false},"stages":{}})","rotatesWithObject");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[],"stages":{"dense":"complete","dense":"failed"}})","duplicate object key");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[],"stages":{"dense":"done"}})","invalid stage state");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[],"stages":{},"calibration":{"width":0,"height":480,"fx":100,"fy":100,"cx":320,"cy":240,"distortion":[]}})","calibration.width");
  const std::string numeric_prefix=R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[],"stages":{},"calibration":{"width":640,"height":480,"fx":100,"fy":100,"cx":)";
  const std::string numeric_suffix=R"(,"cy":240,"distortion":[]}})";
  for (const char *number : {"-0", "0e4000", "-0e-4000", "1e0", "1.7976931348623157e308", "4.9406564584124654e-324"}) {
    auto [numeric_status, numeric_report]=run(numeric_prefix+number+numeric_suffix);
    if (numeric_status != CRISPMESH_OK) {
      std::cerr << "Expected finite JSON number " << number << ", got: " << numeric_report << '\n';
      std::abort();
    }
  }
  for (const char *number : {"1e309", "-1e309", "1e-4000", "-1e-4000"})
    check_invalid(numeric_prefix+number+numeric_suffix,"number out of range");
  for (const char *number : {"01", "1.2.3", "1x", "1,5", "1e+", "-"})
    check_invalid(numeric_prefix+number+numeric_suffix,"");
  const auto original_locale = std::locale();
  std::locale::global(std::locale(std::locale::classic(), new CommaDecimal));
  auto [dot_status, dot_report] = run(numeric_prefix+"1.5"+numeric_suffix);
  std::locale::global(original_locale);
  if (dot_status != CRISPMESH_OK) {
    std::cerr << "Expected JSON dot decimal under comma locale, got: " << dot_report << '\n';
    std::abort();
  }
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"   ","units":"mm","images":[],"stages":{}})","whitespace only");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"\u00a0","units":"mm","images":[],"stages":{}})","whitespace only");
  check_invalid(R"({"schemaVersion":1,"id":"x","name":"x","units":"mm","images":[{"id":"a","path":" images/a.jpg"}],"stages":{}})","edge whitespace");
  std::string malformed_utf8=empty;
  malformed_utf8.replace(malformed_utf8.find("Test"),4,std::string(1,static_cast<char>(0xff)));
  check_invalid(malformed_utf8,"UTF-8");
  size_t required=0;
  assert(crispmesh_geometry_diagnostic_json(nullptr,0,&required)==CRISPMESH_BUFFER_TOO_SMALL);
  std::vector<char> short_buffer(required, '#');
  assert(crispmesh_geometry_diagnostic_json(short_buffer.data(),required-1,&required)==CRISPMESH_BUFFER_TOO_SMALL);
  assert(short_buffer.front()=='#');
  std::vector<char> diagnostic(required);
  assert(crispmesh_geometry_diagnostic_json(diagnostic.data(),diagnostic.size(),&required)==CRISPMESH_OK);
  assert(std::strstr(diagnostic.data(),"\"ok\":true")!=nullptr);
  assert(crispmesh_capabilities_json(nullptr,0,nullptr)==CRISPMESH_INVALID_ARGUMENT);
  char *sparse_report=nullptr;
  size_t sparse_size=0;
  assert(crispmesh_reconstruct_sparse_alloc(nullptr,&sparse_report,&sparse_size)==CRISPMESH_INVALID_ARGUMENT);
  assert(crispmesh_reconstruct_sparse_alloc("",&sparse_report,&sparse_size)==CRISPMESH_INVALID_ARGUMENT);
#ifndef CRISPMESH_WITH_OPENCV
  assert(crispmesh_reconstruct_sparse_alloc("missing-project.json",&sparse_report,&sparse_size)==CRISPMESH_UNAVAILABLE);
  assert(sparse_report && sparse_size==std::strlen(sparse_report)+1);
  assert(std::strstr(sparse_report,"\"ok\":false")!=nullptr);
  crispmesh_free(sparse_report);
#endif
  std::cout << "core contract tests passed\n";
}
