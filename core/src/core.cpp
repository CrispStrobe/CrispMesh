#include "crispmesh/core.h"

#include <algorithm>
#include <array>
#include <clocale>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <limits>
#include <locale.h>
#include <numeric>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>

#ifdef CRISPMESH_WITH_OPENCV
#include <opencv2/calib3d.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/features2d.hpp>
#include <opencv2/objdetect/aruco_detector.hpp>
#endif

namespace {

struct Value {
  enum Kind { null_value, boolean, number, string, array, object } kind = null_value;
  bool b = false;
  double n = 0;
  std::string s;
  std::vector<Value> a;
  std::map<std::string, Value> o;
};

class ParseError : public std::runtime_error {
 public:
  using std::runtime_error::runtime_error;
};

class Parser {
 public:
  Parser(const char *data, size_t size) : data_(data), size_(size) {}
  Value parse() {
    Value result = value(0);
    space();
    if (position_ != size_) fail("trailing content");
    return result;
  }

 private:
  const char *data_;
  size_t size_;
  size_t position_ = 0;

  [[noreturn]] void fail(const char *message) const {
    throw ParseError(std::string(message) + " at byte " + std::to_string(position_));
  }
  void space() {
    while (position_ < size_ && (data_[position_] == ' ' || data_[position_] == '\n' ||
           data_[position_] == '\r' || data_[position_] == '\t')) ++position_;
  }
  bool take(char c) {
    space();
    if (position_ < size_ && data_[position_] == c) { ++position_; return true; }
    return false;
  }
  char next() {
    if (position_ == size_) fail("unexpected end of JSON");
    return data_[position_++];
  }
  void literal(std::string_view text) {
    for (char c : text) if (next() != c) fail("invalid literal");
  }
  static void append_utf8(std::string &out, unsigned cp) {
    if (cp <= 0x7f) out += static_cast<char>(cp);
    else if (cp <= 0x7ff) {
      out += static_cast<char>(0xc0 | (cp >> 6));
      out += static_cast<char>(0x80 | (cp & 0x3f));
    } else if (cp <= 0xffff) {
      out += static_cast<char>(0xe0 | (cp >> 12));
      out += static_cast<char>(0x80 | ((cp >> 6) & 0x3f));
      out += static_cast<char>(0x80 | (cp & 0x3f));
    } else {
      out += static_cast<char>(0xf0 | (cp >> 18));
      out += static_cast<char>(0x80 | ((cp >> 12) & 0x3f));
      out += static_cast<char>(0x80 | ((cp >> 6) & 0x3f));
      out += static_cast<char>(0x80 | (cp & 0x3f));
    }
  }
  unsigned hex4() {
    unsigned cp = 0;
    for (int i = 0; i < 4; ++i) {
      char c = next();
      cp <<= 4;
      if (c >= '0' && c <= '9') cp |= c - '0';
      else if (c >= 'a' && c <= 'f') cp |= c - 'a' + 10;
      else if (c >= 'A' && c <= 'F') cp |= c - 'A' + 10;
      else fail("invalid unicode escape");
    }
    return cp;
  }
  std::string quoted() {
    if (next() != '"') fail("expected string");
    std::string out;
    while (true) {
      char c = next();
      if (c == '"') break;
      if (static_cast<unsigned char>(c) < 0x20) fail("control character in string");
      if (c != '\\') { out += c; continue; }
      switch (next()) {
        case '"': out += '"'; break;
        case '\\': out += '\\'; break;
        case '/': out += '/'; break;
        case 'b': out += '\b'; break;
        case 'f': out += '\f'; break;
        case 'n': out += '\n'; break;
        case 'r': out += '\r'; break;
        case 't': out += '\t'; break;
        case 'u': {
          unsigned cp = hex4();
          if (cp >= 0xd800 && cp <= 0xdbff) {
            if (next() != '\\' || next() != 'u') fail("missing low surrogate");
            unsigned low = hex4();
            if (low < 0xdc00 || low > 0xdfff) fail("invalid low surrogate");
            cp = 0x10000 + ((cp - 0xd800) << 10) + low - 0xdc00;
          } else if (cp >= 0xdc00 && cp <= 0xdfff) fail("unpaired low surrogate");
          append_utf8(out, cp);
          break;
        }
        default: fail("invalid string escape");
      }
    }
    return out;
  }
  Value value(unsigned depth) {
    if (depth > 64) fail("JSON nesting limit exceeded");
    space();
    if (position_ == size_) fail("unexpected end of JSON");
    Value v;
    const char c = data_[position_];
    if (c == '"') { v.kind = Value::string; v.s = quoted(); }
    else if (c == '{') {
      v.kind = Value::object; ++position_;
      if (!take('}')) {
        do {
          space();
          if (position_ == size_ || data_[position_] != '"') fail("expected object key");
          auto key = quoted();
          if (!take(':')) fail("expected colon");
          auto inserted = v.o.emplace(std::move(key), value(depth + 1));
          if (!inserted.second) fail("duplicate object key");
          if (take('}')) break;
          if (!take(',')) fail("expected comma");
        } while (true);
      }
    } else if (c == '[') {
      v.kind = Value::array; ++position_;
      if (!take(']')) {
        do {
          v.a.push_back(value(depth + 1));
          if (take(']')) break;
          if (!take(',')) fail("expected comma");
        } while (true);
      }
    } else if (c == 't') { literal("true"); v.kind = Value::boolean; v.b = true; }
    else if (c == 'f') { literal("false"); v.kind = Value::boolean; }
    else if (c == 'n') { literal("null"); }
    else if (c == '-' || (c >= '0' && c <= '9')) {
      size_t start = position_;
      if (c == '-') ++position_;
      if (position_ == size_) fail("invalid number");
      if (data_[position_] == '0') ++position_;
      else if (data_[position_] >= '1' && data_[position_] <= '9') {
        while (position_ < size_ && data_[position_] >= '0' && data_[position_] <= '9') ++position_;
      } else fail("invalid number");
      if (position_ < size_ && data_[position_] == '.') {
        ++position_;
        if (position_ == size_ || data_[position_] < '0' || data_[position_] > '9') fail("invalid fraction");
        while (position_ < size_ && data_[position_] >= '0' && data_[position_] <= '9') ++position_;
      }
      if (position_ < size_ && (data_[position_] == 'e' || data_[position_] == 'E')) {
        ++position_;
        if (position_ < size_ && (data_[position_] == '+' || data_[position_] == '-')) ++position_;
        if (position_ == size_ || data_[position_] < '0' || data_[position_] > '9') fail("invalid exponent");
        while (position_ < size_ && data_[position_] >= '0' && data_[position_] <= '9') ++position_;
      }
      // Floating-point from_chars is absent in the Apple libc++ used by CI.
      // Use a per-call C numeric locale, never the process-wide mutable locale.
      // Grammar was checked above, and the returned end pointer checks conversion.
      std::string token(data_ + start, position_ - start);
      char *end = nullptr;
#ifdef _WIN32
      _locale_t numeric_locale = _create_locale(LC_NUMERIC, "C");
      if (!numeric_locale) fail("number out of range");
      v.n = _strtod_l(token.c_str(), &end, numeric_locale);
      _free_locale(numeric_locale);
#else
      locale_t numeric_locale = newlocale(LC_NUMERIC_MASK, "C", nullptr);
      if (!numeric_locale) fail("number out of range");
      v.n = strtod_l(token.c_str(), &end, numeric_locale);
      freelocale(numeric_locale);
#endif
      if (end != token.c_str() + token.size() || !std::isfinite(v.n)) fail("number out of range");
      // strtod_l may round tiny nonzero decimals down to zero without failure.
      if (v.n == 0) {
        bool nonzero_mantissa = false;
        for (size_t i = start; i < position_ && data_[i] != 'e' && data_[i] != 'E'; ++i)
          nonzero_mantissa |= data_[i] >= '1' && data_[i] <= '9';
        if (nonzero_mantissa) fail("number out of range");
      }
      v.kind = Value::number;
    } else fail("invalid JSON value");
    return v;
  }
};

bool valid_utf8(const char *data, size_t length) {
  for (size_t i = 0; i < length;) {
    unsigned char c = static_cast<unsigned char>(data[i]);
    if (c < 0x80) { ++i; continue; }
    unsigned codepoint = 0;
    size_t count = 0;
    unsigned minimum = 0;
    if (c >= 0xc2 && c <= 0xdf) { count = 2; codepoint = c & 0x1f; minimum = 0x80; }
    else if (c >= 0xe0 && c <= 0xef) { count = 3; codepoint = c & 0x0f; minimum = 0x800; }
    else if (c >= 0xf0 && c <= 0xf4) { count = 4; codepoint = c & 0x07; minimum = 0x10000; }
    else return false;
    if (count > length - i) return false;
    for (size_t j = 1; j < count; ++j) {
      unsigned char continuation = static_cast<unsigned char>(data[i + j]);
      if ((continuation & 0xc0) != 0x80) return false;
      codepoint = (codepoint << 6) | (continuation & 0x3f);
    }
    if (codepoint < minimum || codepoint > 0x10ffff ||
        (codepoint >= 0xd800 && codepoint <= 0xdfff)) return false;
    i += count;
  }
  return true;
}

unsigned utf8_codepoint(std::string_view s, size_t &index) {
  unsigned char first = static_cast<unsigned char>(s[index++]);
  if (first < 0x80) return first;
  unsigned count = first < 0xe0 ? 2 : first < 0xf0 ? 3 : 4;
  unsigned cp = first & (count == 2 ? 0x1f : count == 3 ? 0x0f : 0x07);
  for (unsigned j = 1; j < count; ++j) cp = (cp << 6) | (static_cast<unsigned char>(s[index++]) & 0x3f);
  return cp;
}
bool js_whitespace(unsigned cp) {
  return (cp >= 0x09 && cp <= 0x0d) || cp == 0x20 || cp == 0xa0 || cp == 0x1680 ||
         (cp >= 0x2000 && cp <= 0x200a) || cp == 0x2028 || cp == 0x2029 ||
         cp == 0x202f || cp == 0x205f || cp == 0x3000 || cp == 0xfeff;
}
bool all_whitespace(std::string_view s) {
  for (size_t i = 0; i < s.size();) if (!js_whitespace(utf8_codepoint(s, i))) return false;
  return true;
}
bool edge_whitespace(std::string_view s) {
  if (s.empty()) return false;
  size_t first = 0;
  unsigned cp = utf8_codepoint(s, first);
  if (js_whitespace(cp)) return true;
  for (size_t i = first; i < s.size();) {
    cp = utf8_codepoint(s, i);
  }
  return js_whitespace(cp);
}

std::string escape(std::string_view s) {
  std::string out = "\"";
  for (unsigned char c : s) {
    switch (c) {
      case '"': out += "\\\""; break;
      case '\\': out += "\\\\"; break;
      case '\n': out += "\\n"; break;
      case '\r': out += "\\r"; break;
      case '\t': out += "\\t"; break;
      default:
        if (c < 0x20) {
          char buf[7]; std::snprintf(buf, sizeof buf, "\\u%04x", c); out += buf;
        } else out += static_cast<char>(c);
    }
  }
  return out + '"';
}

void require(bool condition, const std::string &message) {
  if (!condition) throw ParseError(message);
}
const Value &field(const Value &v, const char *name) {
  auto it = v.o.find(name);
  require(it != v.o.end(), std::string("missing field: ") + name);
  return it->second;
}
void keys(const Value &v, std::initializer_list<std::string_view> allowed, std::string_view at) {
  require(v.kind == Value::object, std::string(at) + " must be an object");
  for (const auto &[key, ignored] : v.o) {
    (void)ignored;
    require(std::find(allowed.begin(), allowed.end(), key) != allowed.end(),
            std::string(at) + " has unknown field: " + key);
  }
}
const std::string &str(const Value &v, std::string_view at) {
  require(v.kind == Value::string, std::string(at) + " must be a string");
  return v.s;
}
void nonempty(const Value &v, std::string_view at) {
  const auto &s = str(v, at);
  require(!all_whitespace(s),
          std::string(at) + " must not be empty or whitespace only");
}
double num(const Value &v, std::string_view at) {
  require(v.kind == Value::number && std::isfinite(v.n), std::string(at) + " must be a finite number");
  return v.n;
}
void positive(const Value &v, std::string_view at) {
  require(num(v, at) > 0, std::string(at) + " must be positive");
}
void dimension(const Value &v, std::string_view at) {
  double n = num(v, at);
  require(n >= 1 && std::floor(n) == n, std::string(at) + " must be a positive integer");
}
void relative_path(const Value &v, std::string_view at) {
  const auto &path = str(v, at);
  require(!path.empty(), std::string(at) + " must be a nonempty relative path");
  require(!edge_whitespace(path), std::string(at) + " must not have edge whitespace");
  require(path[0] != '/' && path[0] != '\\' && path.find('\\') == std::string::npos &&
          path.find(':') == std::string::npos, std::string(at) + " must use relative / separators");
  require(std::none_of(path.begin(), path.end(), [](unsigned char c) { return c < 0x20; }),
          std::string(at) + " contains a control character");
  size_t start = 0;
  while (start < path.size()) {
    size_t end = path.find('/', start);
    if (end == std::string::npos) end = path.size();
    auto part = std::string_view(path).substr(start, end - start);
    require(!part.empty() && part != "." && part != "..", std::string(at) + " contains an unsafe path segment");
    start = end + 1;
  }
  require(path.back() != '/', std::string(at) + " must name a file");
}
void validate(const Value &p) {
  require(p.kind == Value::object, "project must be an object");
  require(num(field(p, "schemaVersion"), "schemaVersion") == 1, "unsupported schemaVersion");
  nonempty(field(p, "id"), "id");
  nonempty(field(p, "name"), "name");
  require(str(field(p, "units"), "units") == "mm", "units must be mm");
  const auto &images = field(p, "images");
  require(images.kind == Value::array, "images must be an array");
  std::map<std::string, bool> ids;
  for (const auto &image : images.a) {
    require(image.kind == Value::object, "image must be an object");
    nonempty(field(image, "id"), "image.id");
    require(ids.emplace(field(image, "id").s, true).second, "image IDs must be unique");
    relative_path(field(image, "path"), "image.path");
    if (image.o.contains("maskPath")) relative_path(field(image, "maskPath"), "image.maskPath");
  }
  if (p.o.contains("calibration")) {
    const auto &c = field(p, "calibration");
    require(c.kind == Value::object, "calibration must be an object");
    dimension(field(c, "width"), "calibration.width");
    dimension(field(c, "height"), "calibration.height");
    positive(field(c, "fx"), "calibration.fx");
    positive(field(c, "fy"), "calibration.fy");
    num(field(c, "cx"), "calibration.cx");
    num(field(c, "cy"), "calibration.cy");
    const auto &d = field(c, "distortion");
    require(d.kind == Value::array, "calibration.distortion must be an array");
    for (const auto &v : d.a) num(v, "calibration.distortion entry");
    if (c.o.contains("distortionModel")) {
      require(str(field(c, "distortionModel"), "calibration.distortionModel") == "opencv-radtan",
              "calibration.distortionModel must be opencv-radtan");
      require(d.a.size() == 4 || d.a.size() == 5 || d.a.size() == 8,
              "calibration.distortion requires 4, 5 or 8 coefficients for opencv-radtan");
    }
  }
  if (p.o.contains("board")) {
    const auto &b = field(p, "board");
    require(b.kind == Value::object, "board must be an object");
    const auto &type = str(field(b, "type"), "board.type");
    require(type == "aruco" || type == "apriltag", "board.type must be aruco or apriltag");
    positive(field(b, "markerSizeMm"), "board.markerSizeMm");
    const auto &rotates = field(b, "rotatesWithObject");
    require(rotates.kind == Value::boolean && rotates.b, "board.rotatesWithObject must be true");
    if (b.o.contains("dictionary")) {
      require(str(field(b, "dictionary"), "board.dictionary") == "DICT_4X4_50",
              "board.dictionary must be DICT_4X4_50");
      require(type == "aruco", "board.dictionary requires aruco");
    }
    if (b.o.contains("markers")) {
      require(type == "aruco" && b.o.contains("dictionary"),
              "board.markers requires aruco and board.dictionary");
      const auto &markers = field(b, "markers");
      require(markers.kind == Value::array && !markers.a.empty(),
              "board.markers must contain at least one marker");
      std::set<int> marker_ids;
      const double size = field(b, "markerSizeMm").n;
      for (const auto &marker : markers.a) {
        require(marker.kind == Value::object, "board.marker must be an object");
        double id = num(field(marker, "id"), "board.marker.id");
        require(id >= 0 && id <= 49 && std::floor(id) == id,
                "board.marker.id must be an integer from 0 to 49");
        require(marker_ids.insert(static_cast<int>(id)).second,
                "board.marker IDs must be unique");
        const auto &corners = field(marker, "cornersMm");
        require(corners.kind == Value::array && corners.a.size() == 4,
                "board.marker.cornersMm must have four corners");
        std::array<std::array<double, 2>, 4> xy{};
        for (size_t i = 0; i < 4; ++i) {
          const auto &corner = corners.a[i];
          require(corner.kind == Value::array && corner.a.size() == 3,
                  "board.marker corner must be [x,y,z]");
          xy[i] = {num(corner.a[0], "board.marker corner x"),
                   num(corner.a[1], "board.marker corner y")};
          require(num(corner.a[2], "board.marker corner z") == 0,
                  "board.marker corners must lie at z=0");
        }
        auto edge = [&](size_t i) {
          return std::array<double, 2>{xy[(i + 1) % 4][0] - xy[i][0],
                                       xy[(i + 1) % 4][1] - xy[i][1]};
        };
        for (size_t i = 0; i < 4; ++i) {
          auto e = edge(i);
          double length = std::hypot(e[0], e[1]);
          require(std::abs(length - size) <= std::max(1e-8, size * 1e-4),
                  "board.marker corners must form markerSizeMm square");
          auto next = edge((i + 1) % 4);
          require(std::abs(e[0] * next[0] + e[1] * next[1]) <= size * std::max(1e-8, size * 1e-4),
                  "board.marker corners must form markerSizeMm square");
          require(e[0] * next[1] - e[1] * next[0] > size * size * (1 - 1e-4),
                  "board.marker corners must have positive winding");
        }
      }
    }
  }
  const auto &stages = field(p, "stages");
  keys(stages, {"calibration", "poses", "sparse", "dense", "mesh", "export"}, "stages");
  for (const auto &[name, value] : stages.o) {
    const auto &state = str(value, "stage state");
    require(state == "pending" || state == "running" || state == "complete" ||
            state == "failed" || state == "unavailable", "invalid stage state for " + name);
  }
}

crispmesh_status copy(std::string_view value, char *output, size_t capacity, size_t *required,
                     crispmesh_status status = CRISPMESH_OK) {
  if (!required || (capacity && !output)) return CRISPMESH_INVALID_ARGUMENT;
  *required = value.size() + 1;
  if (capacity < *required) return CRISPMESH_BUFFER_TOO_SMALL;
  std::memcpy(output, value.data(), value.size());
  output[value.size()] = '\0';
  return status;
}
std::string error(std::string_view code, std::string_view message) {
  return "{\"ok\":false,\"error\":{\"code\":" + escape(code) + ",\"message\":" + escape(message) + "}}";
}

struct Vec3 { double x, y, z; };
Vec3 operator+(Vec3 a, Vec3 b) { return {a.x+b.x,a.y+b.y,a.z+b.z}; }
Vec3 operator-(Vec3 a, Vec3 b) { return {a.x-b.x,a.y-b.y,a.z-b.z}; }
Vec3 operator*(Vec3 a, double s) { return {a.x*s,a.y*s,a.z*s}; }
double dot(Vec3 a, Vec3 b) { return a.x*b.x+a.y*b.y+a.z*b.z; }
Vec3 normalized(Vec3 a) { return a*(1.0/std::sqrt(dot(a,a))); }
std::array<double,2> project(Vec3 world, Vec3 camera_center, double fx, double fy, double cx, double cy) {
  Vec3 c = world-camera_center;
  if (c.z <= 0) throw std::runtime_error("point is behind camera");
  return {fx*c.x/c.z+cx, fy*c.y/c.z+cy};
}
Vec3 ray(std::array<double,2> pixel, double fx, double fy, double cx, double cy) {
  return normalized({(pixel[0]-cx)/fx,(pixel[1]-cy)/fy,1});
}
Vec3 triangulate(Vec3 c1, Vec3 d1, Vec3 c2, Vec3 d2) {
  double b = dot(d1,d2), det = 1-b*b;
  if (det < 1e-8) throw std::runtime_error("insufficient parallax");
  Vec3 baseline = c2-c1;
  double s = (dot(baseline,d1)-b*dot(baseline,d2))/det;
  double t = (b*dot(baseline,d1)-dot(baseline,d2))/det;
  if (s <= 0 || t <= 0) throw std::runtime_error("negative depth");
  return ((c1+d1*s)+(c2+d2*t))*0.5;
}

class PoseError : public std::runtime_error {
 public:
  PoseError(std::string code, std::string message, std::string image_id = {})
      : std::runtime_error(message), code(std::move(code)), image_id(std::move(image_id)) {}
  std::string code, image_id;
};

std::string pose_error(const PoseError &e) {
  return "{\"ok\":false,\"error\":{\"code\":" + escape(e.code) +
         ",\"message\":" + escape(e.what()) +
         (e.image_id.empty() ? "" : ",\"imageId\":" + escape(e.image_id)) + "}}";
}

std::filesystem::path utf8_path(std::string_view text) {
  std::u8string bytes;
  bytes.reserve(text.size());
  for (unsigned char c : text) bytes.push_back(static_cast<char8_t>(c));
  return std::filesystem::path(bytes);
}

std::vector<unsigned char> read_bounded(const std::filesystem::path &path, size_t limit,
                                        const std::string &image_id = {}) {
  std::error_code ec;
  auto size = std::filesystem::file_size(path, ec);
  if (ec) throw PoseError("io_error", "Cannot stat file: " + path.string(), image_id);
  if (size == 0 || size > limit)
    throw PoseError("resource_limit", "File is empty or exceeds size limit: " + path.string(), image_id);
  std::ifstream stream(path, std::ios::binary);
  if (!stream) throw PoseError("io_error", "Cannot open file: " + path.string(), image_id);
  std::vector<unsigned char> bytes(static_cast<size_t>(size));
  if (!stream.read(reinterpret_cast<char *>(bytes.data()), static_cast<std::streamsize>(size)))
    throw PoseError("io_error", "Cannot read file: " + path.string(), image_id);
  return bytes;
}

#ifdef CRISPMESH_WITH_OPENCV
std::pair<int, int> encoded_dimensions(const std::vector<unsigned char> &bytes,
                                       const std::string &image_id) {
  if (bytes.size() >= 24 && std::memcmp(bytes.data(), "\x89PNG\r\n\x1a\n", 8) == 0 &&
      std::memcmp(bytes.data() + 12, "IHDR", 4) == 0) {
    auto u32 = [&](size_t p) { return (uint32_t(bytes[p]) << 24) | (uint32_t(bytes[p+1]) << 16) |
                                      (uint32_t(bytes[p+2]) << 8) | uint32_t(bytes[p+3]); };
    return {static_cast<int>(u32(16)), static_cast<int>(u32(20))};
  }
  if (bytes.size() >= 4 && bytes[0] == 0xff && bytes[1] == 0xd8) {
    size_t p = 2;
    while (p + 4 <= bytes.size()) {
      if (bytes[p++] != 0xff) break;
      while (p < bytes.size() && bytes[p] == 0xff) ++p;
      if (p >= bytes.size()) break;
      unsigned marker = bytes[p++];
      if (marker == 0xd9 || marker == 0xda) break;
      if (marker == 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
      if (p + 2 > bytes.size()) break;
      size_t length = (size_t(bytes[p]) << 8) | bytes[p+1];
      if (length < 2 || p + length > bytes.size()) break;
      if (((marker >= 0xc0 && marker <= 0xcf) && marker != 0xc4 && marker != 0xc8 && marker != 0xcc) && length >= 7)
        return {(int(bytes[p+5]) << 8) | bytes[p+6], (int(bytes[p+3]) << 8) | bytes[p+4]};
      p += length;
    }
  }
  throw PoseError("unsupported_image", "Image must be a valid PNG or JPEG", image_id);
}

std::filesystem::path contained_image(const std::filesystem::path &root,
                                      const std::string &relative, const std::string &image_id) {
  std::error_code ec;
  auto path = std::filesystem::canonical(root / utf8_path(relative), ec);
  if (ec) throw PoseError("io_error", "Cannot resolve image path: " + relative, image_id);
  auto a = root.begin(), b = path.begin();
  for (; a != root.end() && b != path.end() && *a == *b; ++a, ++b) {}
  if (a != root.end() || b == path.end())
    throw PoseError("path_escape", "Image resolves outside project root: " + relative, image_id);
  return path;
}

struct PoseView {
  std::string id, path;
  cv::Mat gray, R, t;
};

std::string estimate_poses(const Value &p, const std::filesystem::path &root,
                           std::vector<PoseView> *views = nullptr) {
  if (!p.o.contains("calibration") || !p.o.contains("board"))
    throw PoseError("pose_prerequisite", "Pose estimation requires calibration and board layout");
  const auto &c = field(p, "calibration"), &b = field(p, "board");
  if (!c.o.contains("distortionModel") || str(field(c, "distortionModel"), "distortionModel") != "opencv-radtan")
    throw PoseError("pose_prerequisite", "Pose estimation requires calibration.distortionModel=opencv-radtan");
  if (str(field(b, "type"), "board.type") != "aruco" ||
      !b.o.contains("dictionary") || !b.o.contains("markers"))
    throw PoseError("pose_prerequisite", "Pose estimation requires full DICT_4X4_50 ArUco board layout");
  const auto &images = field(p, "images").a;
  if (images.empty()) throw PoseError("pose_prerequisite", "Pose estimation requires images");
  double raw_width = field(c, "width").n, raw_height = field(c, "height").n;
  if (raw_width > 10000 || raw_height > 10000 || raw_width * raw_height > 40000000)
    throw PoseError("resource_limit", "Calibration dimensions exceed 40 megapixels or 10000 pixels per axis");
  int width = static_cast<int>(raw_width), height = static_cast<int>(raw_height);
  cv::Mat K = (cv::Mat_<double>(3,3) << field(c,"fx").n,0,field(c,"cx").n,
      0,field(c,"fy").n,field(c,"cy").n, 0,0,1);
  const auto &distortion = field(c,"distortion").a;
  cv::Mat D(1, static_cast<int>(distortion.size()), CV_64F);
  for (size_t i = 0; i < distortion.size(); ++i) D.at<double>(0, static_cast<int>(i)) = distortion[i].n;
  std::map<int, std::array<cv::Point3f,4>> layout;
  for (const auto &marker : field(b,"markers").a) {
    std::array<cv::Point3f,4> points;
    for (size_t j = 0; j < 4; ++j) {
      const auto &v = field(marker,"cornersMm").a[j].a;
      if (std::abs(v[0].n) > 1000000 || std::abs(v[1].n) > 1000000)
        throw PoseError("resource_limit", "Board coordinates exceed one million millimetres");
      points[j] = cv::Point3f(static_cast<float>(v[0].n),static_cast<float>(v[1].n),0);
    }
    layout.emplace(static_cast<int>(field(marker,"id").n), points);
  }
  cv::aruco::ArucoDetector detector(cv::aruco::getPredefinedDictionary(cv::aruco::DICT_4X4_50));
  std::ostringstream out;
  out << std::setprecision(12) << "{\"ok\":true,\"poseSchemaVersion\":1,\"projectId\":"
      << escape(field(p,"id").s) << ",\"units\":\"mm\","
      << "\"convention\":\"object-to-camera\",\"matrixStorage\":\"row-major\","
      << "\"backend\":{\"name\":\"OpenCV\",\"version\":" << escape(CV_VERSION)
      << ",\"dictionary\":\"DICT_4X4_50\",\"detector\":\"ArucoDetector/default-parameters\","
      << "\"solver\":\"IPPE+LM\",\"poseAmbiguityMarginPx\":0.25,"
      << "\"maxRmsPx\":3,\"maxErrorPx\":8},\"poses\":[";
  bool first_image = true;
  for (const auto &image : images) {
    std::string image_id = field(image,"id").s;
    auto path = contained_image(root, field(image,"path").s, image_id);
    auto bytes = read_bounded(path, 64 * 1024 * 1024, image_id);
    auto [encoded_width, encoded_height] = encoded_dimensions(bytes, image_id);
    if (encoded_width != width || encoded_height != height)
      throw PoseError("image_dimensions", "Image dimensions differ from calibration", image_id);
    cv::Mat decoded = cv::imdecode(bytes, cv::IMREAD_GRAYSCALE | cv::IMREAD_IGNORE_ORIENTATION);
    if (decoded.empty() || decoded.cols != width || decoded.rows != height)
      throw PoseError("image_decode", "Cannot decode image at calibration dimensions", image_id);
    std::vector<std::vector<cv::Point2f>> corners;
    std::vector<int> ids;
    detector.detectMarkers(decoded, corners, ids);
    std::vector<cv::Point3f> object_points;
    std::vector<cv::Point2f> image_points;
    std::vector<int> used_ids;
    std::vector<int> ignored_ids;
    std::set<int> seen_ids;
    for (size_t i = 0; i < ids.size(); ++i) {
      auto found = layout.find(ids[i]);
      if (found == layout.end() || corners[i].size() != 4) { ignored_ids.push_back(ids[i]); continue; }
      if (!seen_ids.insert(ids[i]).second)
        throw PoseError("duplicate_marker", "Configured marker ID appears more than once in image", image_id);
      used_ids.push_back(ids[i]);
      for (size_t j = 0; j < 4; ++j) {
        object_points.push_back(found->second[j]);
        image_points.push_back(corners[i][j]);
      }
    }
    if (object_points.size() < 4)
      throw PoseError("insufficient_markers", "No configured board marker was detected", image_id);
    std::vector<cv::Mat> rvecs, tvecs;
    if (cv::solvePnPGeneric(object_points,image_points,K,D,rvecs,tvecs,false,cv::SOLVEPNP_IPPE) < 1)
      throw PoseError("pose_degenerate", "Planar pose solver failed", image_id);
    auto error_for = [&](const cv::Mat &r, const cv::Mat &t) {
      std::vector<cv::Point2f> projected;
      cv::projectPoints(object_points,r,t,K,D,projected);
      double sum = 0;
      for (size_t i = 0; i < projected.size(); ++i) {
        double e = cv::norm(projected[i]-image_points[i]);
        sum += e*e;
      }
      return std::sqrt(sum/projected.size());
    };
    size_t best = 0;
    double best_error = error_for(rvecs[0],tvecs[0]);
    double second_error = std::numeric_limits<double>::infinity();
    for (size_t i = 1; i < rvecs.size(); ++i) {
      double e = error_for(rvecs[i],tvecs[i]);
      if (e < best_error) { second_error = best_error; best_error = e; best = i; }
      else second_error = std::min(second_error,e);
    }
    if (!std::isfinite(best_error) || second_error - best_error < 0.25)
      throw PoseError("pose_ambiguous", "Planar pose has indistinguishable solutions", image_id);
    cv::Mat rvec = rvecs[best].clone(), tvec = tvecs[best].clone();
    cv::solvePnPRefineLM(object_points,image_points,K,D,rvec,tvec);
    cv::Mat R;
    cv::Rodrigues(rvec,R);
    std::vector<cv::Point2f> projected;
    cv::projectPoints(object_points,rvec,tvec,K,D,projected);
    double sum = 0, maximum = 0;
    for (size_t i = 0; i < projected.size(); ++i) {
      double e = cv::norm(projected[i]-image_points[i]);
      sum += e*e;
      maximum = std::max(maximum,e);
      cv::Mat point = (cv::Mat_<double>(3,1) << object_points[i].x,object_points[i].y,object_points[i].z);
      cv::Mat camera = R*point+tvec;
      if (!std::isfinite(camera.at<double>(2)) || camera.at<double>(2) <= 0)
        throw PoseError("negative_depth", "Board is behind the camera", image_id);
    }
    double rms = std::sqrt(sum/projected.size());
    if (!std::isfinite(rms) || rms > 3 || maximum > 8)
      throw PoseError("excessive_residual", "Board reprojection residual exceeds threshold", image_id);
    if (views) views->push_back({image_id, field(image,"path").s, decoded, R, tvec});
    if (!first_image) out << ',';
    first_image = false;
    out << "{\"imageId\":" << escape(image_id) << ",\"path\":"
        << escape(field(image,"path").s) << ",\"detectedMarkerIds\":[";
    for (size_t i = 0; i < used_ids.size(); ++i) { if (i) out << ','; out << used_ids[i]; }
    out << "],\"ignoredMarkerIds\":[";
    for (size_t i = 0; i < ignored_ids.size(); ++i) { if (i) out << ','; out << ignored_ids[i]; }
    out << "],\"cornerCount\":" << object_points.size() << ",\"rotation\":[";
    for (int i = 0; i < 9; ++i) { if (i) out << ','; out << R.at<double>(i/3,i%3); }
    out << "],\"translationMm\":[" << tvec.at<double>(0) << ',' << tvec.at<double>(1) << ','
        << tvec.at<double>(2) << "],\"rmsReprojectionErrorPx\":" << rms
        << ",\"maxReprojectionErrorPx\":" << maximum << '}';
  }
  out << "]}";
  return out.str();
}

constexpr int sparse_max_views = 64;
constexpr uint64_t sparse_max_total_pixels = 256000000;
constexpr int sparse_max_features = 1500;
constexpr int sparse_pair_window = 2;
constexpr int sparse_mask_radius = 32;
constexpr size_t sparse_max_report_bytes = 16*1024*1024;
constexpr double sparse_max_coordinate_mm = 1000000;
constexpr double sparse_ratio = 0.75;
constexpr double sparse_min_parallax = 1.0;
constexpr double sparse_max_reprojection = 2.0;

struct SparseView {
  std::string mask_path;
  std::vector<cv::KeyPoint> keypoints;
  cv::Mat descriptors;
  std::vector<cv::Point2f> undistorted;
  Vec3 center;
  std::vector<Vec3> rays;
};
struct SparseEdge { int a, b, distance; };
struct SparsePoint {
  Vec3 position;
  double rms, maximum, parallax;
  std::vector<std::pair<int,int>> observations;
};
Vec3 cv_vec(const cv::Mat &m) { return {m.at<double>(0), m.at<double>(1), m.at<double>(2)}; }
Vec3 world_ray(const cv::Mat &R, const cv::Point2f &p) {
  cv::Mat d = (cv::Mat_<double>(3,1) << p.x,p.y,1.0);
  return normalized(cv_vec(R.t()*d));
}
double norm3(Vec3 v) { return std::sqrt(dot(v,v)); }
double parallax_deg(Vec3 a, Vec3 b) {
  double angle=std::acos(std::clamp(dot(a,b),-1.0,1.0))*180.0/CV_PI;
  return std::min(angle,180.0-angle);
}
bool finite_point(Vec3 p) { return std::isfinite(p.x) && std::isfinite(p.y) && std::isfinite(p.z); }

std::string reconstruct_sparse(const Value &project, const std::filesystem::path &root) {
  const auto &images = field(project,"images").a;
  if (field(project,"id").s.size()>256)
    throw PoseError("resource_limit","Sparse project ID exceeds 256 bytes");
  if (images.size() < 2) throw PoseError("sparse_prerequisite", "Sparse reconstruction requires two or more images");
  if (images.size() > sparse_max_views) throw PoseError("resource_limit", "Sparse reconstruction exceeds 64 views");
  for (const auto &image : images) {
    if (field(image,"id").s.size()>256 || field(image,"path").s.size()>4096 ||
        (image.o.contains("maskPath") && field(image,"maskPath").s.size()>4096))
      throw PoseError("resource_limit","Sparse image ID or path exceeds length limit",field(image,"id").s);
    if (!image.o.contains("maskPath"))
      throw PoseError("missing_mask", "Every sparse view requires maskPath",field(image,"id").s);
  }
  std::vector<PoseView> poses;
  estimate_poses(project,root,&poses);
  for (const auto &pose:poses) for (int k=0;k<3;++k)
    if (!std::isfinite(pose.t.at<double>(k)) || std::abs(pose.t.at<double>(k))>sparse_max_coordinate_mm)
      throw PoseError("resource_limit","Sparse pose translation exceeds coordinate range",pose.id);
  const auto &cal = field(project,"calibration");
  if (field(cal,"width").n*field(cal,"height").n*images.size()>sparse_max_total_pixels)
    throw PoseError("resource_limit","Sparse view pixels exceed aggregate 256 megapixel limit");
  cv::Mat K = (cv::Mat_<double>(3,3) << field(cal,"fx").n,0,field(cal,"cx").n,
      0,field(cal,"fy").n,field(cal,"cy").n, 0,0,1);
  const auto &coefficients = field(cal,"distortion").a;
  cv::Mat D(1,static_cast<int>(coefficients.size()),CV_64F);
  for (size_t i=0;i<coefficients.size();++i) D.at<double>(0,static_cast<int>(i))=coefficients[i].n;
  int width=static_cast<int>(field(cal,"width").n), height=static_cast<int>(field(cal,"height").n);
  auto orb=cv::ORB::create(sparse_max_features,1.2f,1,32,0,2,cv::ORB::HARRIS_SCORE,31,20);
  std::vector<SparseView> views(poses.size());
  for (size_t i=0;i<poses.size();++i) {
    auto &view=views[i];
    view.mask_path=field(images[i],"maskPath").s;
    std::filesystem::path path;
    try { path=contained_image(root,view.mask_path,poses[i].id); }
    catch (const PoseError &e) {
      if (e.code=="io_error") throw PoseError("missing_mask","Cannot resolve mask path: "+view.mask_path,poses[i].id);
      throw;
    }
    std::vector<unsigned char> bytes;
    try { bytes=read_bounded(path,64*1024*1024,poses[i].id); }
    catch (const PoseError &e) {
      if (e.code=="io_error") throw PoseError("missing_mask","Cannot read mask: "+view.mask_path,poses[i].id);
      throw;
    }
    if (bytes.size()<26 || std::memcmp(bytes.data(),"\x89PNG\r\n\x1a\n",8)!=0 ||
        std::memcmp(bytes.data()+12,"IHDR",4)!=0 || bytes[24]!=8 || bytes[25]!=0)
      throw PoseError("invalid_mask", "Mask must be an 8-bit grayscale PNG",poses[i].id);
    auto [mw,mh]=encoded_dimensions(bytes,poses[i].id);
    if (mw!=width || mh!=height) throw PoseError("mask_dimensions", "Mask dimensions differ from calibration",poses[i].id);
    cv::Mat mask=cv::imdecode(bytes,cv::IMREAD_UNCHANGED);
    if (mask.empty() || mask.type()!=CV_8UC1 || mask.size()!=poses[i].gray.size())
      throw PoseError("invalid_mask", "Cannot decode grayscale mask",poses[i].id);
    cv::Mat invalid;
    cv::compare(mask,0,invalid,cv::CMP_NE);
    cv::Mat nonbinary;
    cv::compare(mask,255,nonbinary,cv::CMP_NE);
    cv::bitwise_and(invalid,nonbinary,nonbinary);
    if (cv::countNonZero(nonbinary)) throw PoseError("invalid_mask", "Mask pixels must be 0 or 255",poses[i].id);
    for (const auto &marker : field(field(project,"board"),"markers").a) {
      std::vector<cv::Point3f> corners;
      for (const auto &corner : field(marker,"cornersMm").a)
        corners.emplace_back(static_cast<float>(corner.a[0].n),static_cast<float>(corner.a[1].n),0);
      cv::Mat rvec;
      cv::Rodrigues(poses[i].R,rvec);
      std::vector<cv::Point2f> projected;
      cv::projectPoints(corners,rvec,poses[i].t,K,D,projected);
      std::vector<cv::Point> polygon;
      for (auto p:projected) polygon.emplace_back(cvRound(p.x),cvRound(p.y));
      cv::fillConvexPoly(mask,polygon,cv::Scalar(0));
    }
    cv::erode(mask,mask,cv::getStructuringElement(cv::MORPH_ELLIPSE,
              cv::Size(2*sparse_mask_radius+1,2*sparse_mask_radius+1)),
              cv::Point(-1,-1),1,cv::BORDER_CONSTANT,cv::Scalar(0));
    orb->detectAndCompute(poses[i].gray,mask,view.keypoints,view.descriptors);
    if (!view.keypoints.empty()) {
      std::vector<cv::Point2f> pixels;
      for (auto &kp:view.keypoints) pixels.push_back(kp.pt);
      cv::undistortPoints(pixels,view.undistorted,K,D);
      view.rays.reserve(pixels.size());
      for (auto p:view.undistorted) view.rays.push_back(world_ray(poses[i].R,p));
    }
    cv::Mat center=-poses[i].R.t()*poses[i].t;
    view.center=cv_vec(center);
    poses[i].gray.release();
  }
  int total_features=0;
  for (const auto &v:views) total_features+=static_cast<int>(v.keypoints.size());
  if (!total_features) throw PoseError("no_features","No usable masked ORB features");
  cv::BFMatcher matcher(cv::NORM_HAMMING);
  std::vector<int> offsets(views.size()+1);
  for (size_t i=0;i<views.size();++i) offsets[i+1]=offsets[i]+static_cast<int>(views[i].keypoints.size());
  std::vector<SparseEdge> edges;
  int candidate_matches=0, geometric_matches=0, pairs_tried=0;
  auto projection_error=[&](Vec3 point,int vi,int ki) {
    std::vector<cv::Point3d> xyz{{point.x,point.y,point.z}};
    std::vector<cv::Point2d> pixel;
    cv::Mat rvec;
    cv::Rodrigues(poses[vi].R,rvec);
    cv::projectPoints(xyz,rvec,poses[vi].t,K,D,pixel);
    auto expected=views[vi].keypoints[ki].pt;
    return std::hypot(pixel[0].x-expected.x,pixel[0].y-expected.y);
  };
  auto depth=[&](Vec3 point,int vi) {
    cv::Mat xyz=(cv::Mat_<double>(3,1)<<point.x,point.y,point.z);
    cv::Mat camera=poses[vi].R*xyz+poses[vi].t;
    return camera.at<double>(2);
  };
  for (size_t i=0;i<views.size();++i) for (size_t j=i+1;j<views.size() && j<=i+sparse_pair_window;++j) {
    ++pairs_tried;
    if (views[i].descriptors.rows<2 || views[j].descriptors.rows<2) continue;
    std::vector<std::vector<cv::DMatch>> forward, reverse;
    matcher.knnMatch(views[i].descriptors,views[j].descriptors,forward,2);
    matcher.knnMatch(views[j].descriptors,views[i].descriptors,reverse,2);
    auto good=[](const std::vector<cv::DMatch> &m) {
      return m.size()==2 && m[0].distance<sparse_ratio*m[1].distance;
    };
    for (size_t k=0;k<forward.size();++k) {
      if (!good(forward[k])) continue;
      const auto &m=forward[k][0];
      if (m.trainIdx<0 || static_cast<size_t>(m.trainIdx)>=reverse.size() || !good(reverse[m.trainIdx]) ||
          reverse[m.trainIdx][0].trainIdx!=static_cast<int>(k)) continue;
      ++candidate_matches;
      auto a=views[i].rays[k], b=views[j].rays[m.trainIdx];
      if (parallax_deg(a,b)<sparse_min_parallax) continue;
      Vec3 point;
      try { point=triangulate(views[i].center,a,views[j].center,b); }
      catch (const std::exception &) { continue; }
      if (!finite_point(point) || depth(point,static_cast<int>(i))<=0 || depth(point,static_cast<int>(j))<=0) continue;
      if (projection_error(point,static_cast<int>(i),static_cast<int>(k))>sparse_max_reprojection ||
          projection_error(point,static_cast<int>(j),m.trainIdx)>sparse_max_reprojection) continue;
      ++geometric_matches;
      edges.push_back({offsets[i]+static_cast<int>(k),offsets[j]+m.trainIdx,static_cast<int>(m.distance)});
    }
  }
  if (edges.empty()) {
    if (candidate_matches) throw PoseError("degenerate_baseline","No matches pass baseline, parallax, depth and reprojection checks");
    throw PoseError("no_matches","No geometrically valid masked feature matches");
  }
  std::sort(edges.begin(),edges.end(),[](auto a,auto b) {
    if (a.distance!=b.distance) return a.distance<b.distance;
    if (a.a!=b.a) return a.a<b.a;
    return a.b<b.b;
  });
  std::vector<int> parent(offsets.back());
  std::iota(parent.begin(),parent.end(),0);
  std::vector<uint64_t> image_bits(parent.size());
  for (size_t i=0;i<views.size();++i) for (int k=offsets[i];k<offsets[i+1];++k) image_bits[k]=uint64_t(1)<<i;
  auto find=[&](int a) { while (parent[a]!=a) { parent[a]=parent[parent[a]]; a=parent[a]; } return a; };
  int conflicts=0;
  for (auto e:edges) {
    int a=find(e.a),b=find(e.b);
    if (a==b) continue;
    if (image_bits[a]&image_bits[b]) { ++conflicts; continue; }
    parent[b]=a;
    image_bits[a]|=image_bits[b];
  }
  std::map<int,std::vector<std::pair<int,int>>> groups;
  for (size_t i=0;i<views.size();++i) for (int k=0;k<static_cast<int>(views[i].keypoints.size());++k) {
    int node=offsets[i]+k, root_id=find(node);
    if (image_bits[root_id]&(image_bits[root_id]-1)) groups[root_id].push_back({static_cast<int>(i),k});
  }
  std::vector<SparsePoint> points;
  int rejected_tracks=0;
  for (const auto &[ignored,obs]:groups) {
    (void)ignored;
    double best_angle=0;
    Vec3 best_point{};
    for (size_t a=0;a<obs.size();++a) for (size_t b=a+1;b<obs.size();++b) {
      auto [vi,ki]=obs[a];
      auto [vj,kj]=obs[b];
      double angle=parallax_deg(views[vi].rays[ki],views[vj].rays[kj]);
      if (angle<=best_angle) continue;
      try {
        auto p=triangulate(views[vi].center,views[vi].rays[ki],views[vj].center,views[vj].rays[kj]);
        if (finite_point(p)) { best_point=p; best_angle=angle; }
      } catch (const std::exception &) {}
    }
    if (best_angle<sparse_min_parallax || std::abs(best_point.x)>sparse_max_coordinate_mm ||
        std::abs(best_point.y)>sparse_max_coordinate_mm || std::abs(best_point.z)>sparse_max_coordinate_mm) {
      ++rejected_tracks; continue;
    }
    double sum=0, maximum=0, minimum=180;
    bool valid=true;
    for (auto [vi,ki]:obs) {
      if (depth(best_point,vi)<=0) { valid=false; break; }
      double e=projection_error(best_point,vi,ki);
      if (!std::isfinite(e) || e>sparse_max_reprojection) { valid=false; break; }
      sum+=e*e; maximum=std::max(maximum,e);
    }
    if (!valid) { ++rejected_tracks; continue; }
    for (size_t a=0;a<obs.size();++a) for (size_t b=a+1;b<obs.size();++b)
      minimum=std::min(minimum,parallax_deg(views[obs[a].first].rays[obs[a].second],
                                             views[obs[b].first].rays[obs[b].second]));
    if (minimum<sparse_min_parallax) { ++rejected_tracks; continue; }
    points.push_back({best_point,std::sqrt(sum/obs.size()),maximum,minimum,obs});
  }
  if (points.empty()) throw PoseError("no_points","Sparse reconstruction produced no valid points");
  std::ostringstream out;
  out<<std::setprecision(12)<<"{\"ok\":true,\"sparseSchemaVersion\":1,\"projectId\":"
     <<escape(field(project,"id").s)<<",\"units\":\"mm\",\"coordinateFrame\":\"board\","
     <<"\"backend\":{\"name\":\"OpenCV\",\"version\":"<<escape(CV_VERSION)
     <<",\"feature\":\"ORB\",\"settings\":{\"poseDictionary\":\"DICT_4X4_50\","
     <<"\"poseDetector\":\"ArucoDetector/default-parameters\",\"poseSolver\":\"IPPE+LM\","
     <<"\"poseAmbiguityMarginPx\":0.25,\"poseMaxRmsPx\":3,\"poseMaxErrorPx\":8,"
     <<"\"scaleLevels\":1,\"orbPatchSizePx\":31,\"orbWtaK\":2,\"orbScoreType\":\"HARRIS\","
     <<"\"orbEdgeThresholdPx\":32,\"orbFastThreshold\":20,\"mutualNearest\":true,\"maxViews\":"<<sparse_max_views
     <<",\"maxFeaturesPerView\":"<<sparse_max_features<<",\"pairWindow\":"<<sparse_pair_window
     <<",\"maxPairs\":"<<(2*sparse_max_views-3)<<",\"maxTotalPixels\":"<<sparse_max_total_pixels
     <<",\"maxReportBytes\":"<<sparse_max_report_bytes<<",\"maxCoordinateMm\":"<<sparse_max_coordinate_mm
     <<",\"maxFileBytes\":"<<(64*1024*1024)
     <<",\"maxProjectBytes\":"<<(16*1024*1024)<<",\"maxImagePixels\":40000000,"
     <<"\"maxImageAxisPx\":10000,\"maxIdBytes\":256,\"maxPathBytes\":4096"
     <<",\"maskErosionRadiusPx\":"<<sparse_mask_radius
     <<",\"maskErosionShape\":\"ellipse\",\"projectedBoardExclusion\":true"
     <<",\"matchRatio\":"<<sparse_ratio<<",\"minParallaxDeg\":"<<sparse_min_parallax
     <<",\"maxReprojectionErrorPx\":"<<sparse_max_reprojection<<"}},\"views\":[";
  for (size_t i=0;i<poses.size();++i) {
    if (i) out<<',';
    out<<"{\"imageId\":"<<escape(poses[i].id)<<",\"path\":"<<escape(poses[i].path)
       <<",\"maskPath\":"<<escape(views[i].mask_path)<<",\"featureCount\":"<<views[i].keypoints.size()
       <<",\"rotation\":[";
    for (int k=0;k<9;++k) { if(k) out<<','; out<<poses[i].R.at<double>(k/3,k%3); }
    out<<"],\"translationMm\":["<<poses[i].t.at<double>(0)<<','<<poses[i].t.at<double>(1)<<','
       <<poses[i].t.at<double>(2)<<"]}";
    if (out.tellp()>static_cast<std::streamoff>(sparse_max_report_bytes))
      throw PoseError("resource_limit","Sparse report exceeds 16 MiB");
  }
  out<<"],\"points\":[";
  for (size_t i=0;i<points.size();++i) {
    if (i) out<<',';
    const auto &p=points[i];
    out<<"{\"id\":"<<i<<",\"positionMm\":["<<p.position.x<<','
       <<p.position.y<<','<<p.position.z<<"],\"rmsReprojectionErrorPx\":"<<p.rms
       <<",\"maxReprojectionErrorPx\":"<<p.maximum<<",\"minParallaxDeg\":"<<p.parallax
       <<",\"observations\":[";
    for (size_t j=0;j<p.observations.size();++j) {
      if (j) out<<',';
      auto [vi,ki]=p.observations[j]; auto q=views[vi].keypoints[ki].pt;
      out<<"{\"imageId\":"<<escape(poses[vi].id)<<",\"pixel\":["<<q.x<<','<<q.y<<"]}";
    }
    out<<"]}";
    if (out.tellp()>static_cast<std::streamoff>(sparse_max_report_bytes-4096))
      throw PoseError("resource_limit","Sparse report exceeds 16 MiB");
  }
  out<<"],\"statistics\":{\"candidateMatches\":"<<candidate_matches
     <<",\"acceptedTracks\":"<<points.size()<<",\"rejectedMatches\":"
     <<candidate_matches-geometric_matches+conflicts<<",\"geometricMatches\":"<<geometric_matches
     <<",\"conflictingMatches\":"<<conflicts<<",\"rejectedTracks\":"<<rejected_tracks
     <<",\"pairsTried\":"<<pairs_tried<<"}}";
  if (out.tellp()>static_cast<std::streamoff>(sparse_max_report_bytes))
    throw PoseError("resource_limit","Sparse report exceeds 16 MiB");
  return out.str();
}
#endif

} // namespace

extern "C" uint32_t crispmesh_abi_version(void) { return CRISPMESH_ABI_VERSION; }

extern "C" crispmesh_status crispmesh_capabilities_json(char *out, size_t cap, size_t *needed) {
  return copy("{\"abiVersion\":1,\"projectSchemaVersion\":1,\"geometryDiagnostic\":true,\"projectValidation\":true,\"reconstruction\":{\"available\":false,\"reason\":\"No calibrated reconstruction backend is integrated\"},\"stages\":{\"calibration\":false,\"poses\":"
#ifdef CRISPMESH_WITH_OPENCV
              "true"
#else
              "false"
#endif
              ",\"sparse\":"
#ifdef CRISPMESH_WITH_OPENCV
              "true"
#else
              "false"
#endif
              ",\"dense\":false,\"mesh\":false,\"export\":false}}", out, cap, needed);
}

namespace {
std::pair<crispmesh_status,std::string> pose_result(const char *project_path) {
#ifndef CRISPMESH_WITH_OPENCV
  (void)project_path;
  return {CRISPMESH_UNAVAILABLE, error("unavailable", "Pose estimation requires an OpenCV-enabled build")};
#else
  try {
    std::error_code ec;
    auto path = std::filesystem::canonical(utf8_path(project_path), ec);
    if (ec) throw PoseError("io_error", "Cannot resolve project file");
    auto bytes = read_bounded(path, 16 * 1024 * 1024);
    require(valid_utf8(reinterpret_cast<const char *>(bytes.data()), bytes.size()),
            "project JSON is not valid UTF-8");
    Value project = Parser(reinterpret_cast<const char *>(bytes.data()), bytes.size()).parse();
    validate(project);
    return {CRISPMESH_OK, estimate_poses(project, path.parent_path())};
  } catch (const PoseError &e) {
    return {CRISPMESH_INVALID_PROJECT, pose_error(e)};
  } catch (const ParseError &e) {
    return {CRISPMESH_INVALID_PROJECT, error("invalid_project", e.what())};
  } catch (const cv::Exception &e) {
    return {CRISPMESH_INTERNAL_ERROR, error("backend_error", e.what())};
  } catch (const std::exception &e) {
    return {CRISPMESH_INTERNAL_ERROR, error("internal_error", e.what())};
  }
#endif
}
std::pair<crispmesh_status,std::string> sparse_result(const char *project_path) {
#ifndef CRISPMESH_WITH_OPENCV
  (void)project_path;
  return {CRISPMESH_UNAVAILABLE,error("unavailable","Sparse reconstruction requires an OpenCV-enabled build")};
#else
  try {
    std::error_code ec;
    auto path=std::filesystem::canonical(utf8_path(project_path),ec);
    if (ec) throw PoseError("io_error","Cannot resolve project file");
    auto bytes=read_bounded(path,16*1024*1024);
    require(valid_utf8(reinterpret_cast<const char *>(bytes.data()),bytes.size()),
            "project JSON is not valid UTF-8");
    Value project=Parser(reinterpret_cast<const char *>(bytes.data()),bytes.size()).parse();
    validate(project);
    return {CRISPMESH_OK,reconstruct_sparse(project,path.parent_path())};
  } catch (const PoseError &e) {
    return {CRISPMESH_INVALID_PROJECT,pose_error(e)};
  } catch (const ParseError &e) {
    return {CRISPMESH_INVALID_PROJECT,error("invalid_project",e.what())};
  } catch (const cv::Exception &e) {
    return {CRISPMESH_INTERNAL_ERROR,error("backend_error",e.what())};
  } catch (const std::exception &e) {
    return {CRISPMESH_INTERNAL_ERROR,error("internal_error",e.what())};
  }
#endif
}
} // namespace

extern "C" crispmesh_status crispmesh_estimate_poses_json(const char *project_path,
                                                           char *out, size_t cap, size_t *needed) {
  if (!project_path || !*project_path || !needed) return CRISPMESH_INVALID_ARGUMENT;
  auto [status, value] = pose_result(project_path);
  return copy(value, out, cap, needed, status);
}

extern "C" crispmesh_status crispmesh_estimate_poses_alloc(const char *project_path,
                                                            char **out, size_t *size) {
  if (!project_path || !*project_path || !out || !size) return CRISPMESH_INVALID_ARGUMENT;
  *out = nullptr;
  *size = 0;
  auto [status, value] = pose_result(project_path);
  char *buffer = static_cast<char *>(std::malloc(value.size() + 1));
  if (!buffer) return CRISPMESH_INTERNAL_ERROR;
  std::memcpy(buffer,value.data(),value.size());
  buffer[value.size()] = '\0';
  *out = buffer;
  *size = value.size() + 1;
  return status;
}

extern "C" crispmesh_status crispmesh_reconstruct_sparse_alloc(const char *project_path,
                                                               char **out, size_t *size) {
  if (!project_path || !*project_path || !out || !size) return CRISPMESH_INVALID_ARGUMENT;
  *out=nullptr;
  *size=0;
  auto [status,value]=sparse_result(project_path);
  char *buffer=static_cast<char *>(std::malloc(value.size()+1));
  if (!buffer) return CRISPMESH_INTERNAL_ERROR;
  std::memcpy(buffer,value.data(),value.size());
  buffer[value.size()]='\0';
  *out=buffer;
  *size=value.size()+1;
  return status;
}

extern "C" void crispmesh_free(void *pointer) { std::free(pointer); }

extern "C" crispmesh_status crispmesh_validate_project_json(const char *input, size_t length,
                                                              char *out, size_t cap, size_t *needed) {
  if (!input || length > 16*1024*1024 || !needed) return CRISPMESH_INVALID_ARGUMENT;
  try {
    require(valid_utf8(input, length), "project JSON is not valid UTF-8");
    auto project = Parser(input, length).parse();
    validate(project);
    return copy("{\"ok\":true,\"schemaVersion\":1,\"imageCount\":" +
                std::to_string(project.o.at("images").a.size()) + ",\"reconstructionReady\":false}", out, cap, needed);
  } catch (const ParseError &e) {
    return copy(error("invalid_project", e.what()), out, cap, needed, CRISPMESH_INVALID_PROJECT);
  } catch (const std::exception &e) {
    return copy(error("internal_error", e.what()), out, cap, needed, CRISPMESH_INTERNAL_ERROR);
  }
}

extern "C" crispmesh_status crispmesh_geometry_diagnostic_json(char *out, size_t cap, size_t *needed) {
  try {
    constexpr double fx=1000, fy=980, cx=640, cy=360;
    Vec3 truth{35,-20,620}, c1{0,0,0}, c2{120,0,0};
    auto p1=project(truth,c1,fx,fy,cx,cy), p2=project(truth,c2,fx,fy,cx,cy);
    auto found=triangulate(c1,ray(p1,fx,fy,cx,cy),c2,ray(p2,fx,fy,cx,cy));
    auto q1=project(found,c1,fx,fy,cx,cy), q2=project(found,c2,fx,fy,cx,cy);
    double position=std::sqrt(dot(found-truth,found-truth));
    double reprojection=std::max(std::hypot(q1[0]-p1[0],q1[1]-p1[1]),
                                 std::hypot(q2[0]-p2[0],q2[1]-p2[1]));
    bool passed=position < 1e-6 && reprojection < 1e-6;
    char result[512];
    std::snprintf(result,sizeof result,
      "{\"ok\":%s,\"diagnostic\":\"synthetic_two_view_triangulation\",\"units\":\"mm\",\"baselineMm\":120,\"positionErrorMm\":%.12g,\"maxReprojectionErrorPx\":%.12g}",
      passed ? "true" : "false", position, reprojection);
    return copy(result,out,cap,needed,passed ? CRISPMESH_OK : CRISPMESH_INTERNAL_ERROR);
  } catch (const std::exception &e) {
    return copy(error("geometry_failure",e.what()),out,cap,needed,CRISPMESH_INTERNAL_ERROR);
  }
}

extern "C" crispmesh_status crispmesh_reconstruct_json(const char *input, size_t length,
                                                        char *out, size_t cap, size_t *needed) {
  if (!input || !needed) return CRISPMESH_INVALID_ARGUMENT;
  size_t validation_size=0;
  auto result=crispmesh_validate_project_json(input,length,nullptr,0,&validation_size);
  if (result != CRISPMESH_BUFFER_TOO_SMALL) return result;
  std::string validation(validation_size,'\0');
  result=crispmesh_validate_project_json(input,length,validation.data(),validation.size(),&validation_size);
  if (result != CRISPMESH_OK) return copy(validation.c_str(),out,cap,needed,result);
  return copy(error("unavailable","Reconstruction requires a calibrated backend; no mesh was produced"),
              out,cap,needed,CRISPMESH_UNAVAILABLE);
}
