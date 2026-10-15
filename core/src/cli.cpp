#include "crispmesh/core.h"

#include <fstream>
#include <iostream>
#include <iterator>
#include <string>
#include <vector>

namespace {
using operation = crispmesh_status (*)(char *, size_t, size_t *);
using project_operation = crispmesh_status (*)(const char *, size_t, char *, size_t, size_t *);

int output(operation fn) {
  size_t needed = 0;
  if (fn(nullptr, 0, &needed) != CRISPMESH_BUFFER_TOO_SMALL || needed == 0) return 1;
  std::vector<char> buffer(needed);
  auto status = fn(buffer.data(), buffer.size(), &needed);
  std::cout << buffer.data() << '\n';
  return status == CRISPMESH_OK ? 0 : 1;
}
int output(project_operation fn, const std::string &input) {
  size_t needed = 0;
  if (fn(input.data(), input.size(), nullptr, 0, &needed) != CRISPMESH_BUFFER_TOO_SMALL || needed == 0) return 1;
  std::vector<char> buffer(needed);
  auto status = fn(input.data(), input.size(), buffer.data(), buffer.size(), &needed);
  std::cout << buffer.data() << '\n';
  return status == CRISPMESH_OK ? 0 : status == CRISPMESH_UNAVAILABLE ? 3 : 2;
}
int output_path(const char *path, bool sparse = false) {
  if (!path || !*path) {
    std::cout << "{\"ok\":false,\"error\":{\"code\":\"invalid_argument\",\"message\":\"Project path is empty\"}}\n";
    return 2;
  }
  char *report = nullptr;
  size_t size = 0;
  auto status = sparse ? crispmesh_reconstruct_sparse_alloc(path,&report,&size)
                       : crispmesh_estimate_poses_alloc(path,&report,&size);
  if (!report || !size) return 1;
  std::cout << report << '\n';
  crispmesh_free(report);
  return status == CRISPMESH_OK ? 0 : status == CRISPMESH_UNAVAILABLE ? 3 : 2;
}
void usage() {
  std::cerr << "Usage: crispmesh capabilities | diagnose-geometry | validate <project.json> | estimate-poses <project.json> | reconstruct-sparse <project.json> | reconstruct <project.json>\n";
}
} // namespace

int main(int argc, char **argv) {
  if (argc == 2 && std::string(argv[1]) == "capabilities") return output(crispmesh_capabilities_json);
  if (argc == 2 && std::string(argv[1]) == "diagnose-geometry") return output(crispmesh_geometry_diagnostic_json);
  if (argc != 3) { usage(); return 64; }
  const std::string command = argv[1];
  if (command == "estimate-poses") return output_path(argv[2]);
  if (command == "reconstruct-sparse") return output_path(argv[2],true);
  if (command != "validate" && command != "reconstruct") { usage(); return 64; }
  std::ifstream file(argv[2], std::ios::binary);
  if (!file) {
    std::cout << "{\"ok\":false,\"error\":{\"code\":\"io_error\",\"message\":\"Cannot open project file\"}}\n";
    return 2;
  }
  file.seekg(0, std::ios::end);
  auto length = file.tellg();
  if (length < 0 || length > 16 * 1024 * 1024) {
    std::cout << "{\"ok\":false,\"error\":{\"code\":\"invalid_project\",\"message\":\"Project JSON exceeds 16 MiB\"}}\n";
    return 2;
  }
  file.seekg(0, std::ios::beg);
  std::string input(static_cast<size_t>(length), '\0');
  if (!file.read(input.data(), length)) {
    std::cout << "{\"ok\":false,\"error\":{\"code\":\"io_error\",\"message\":\"Cannot read project file\"}}\n";
    return 2;
  }
  return output(command == "validate" ? crispmesh_validate_project_json : crispmesh_reconstruct_json, input);
}
