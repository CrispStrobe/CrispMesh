# Opt-in OpenCV build and source inventory

`CRISPMESH_WITH_OPENCV` defaults to `OFF`. Enable R01 image geometry with:

```sh
cmake -S . -B build-opencv -DCMAKE_BUILD_TYPE=Release -DCRISPMESH_WITH_OPENCV=ON
cmake --build build-opencv -j4
ctest --test-dir build-opencv --output-on-failure
```

`cmake/OpenCV.cmake` fetches the OpenCV 4.12.0 tag archive from
`https://codeload.github.com/opencv/opencv/tar.gz/refs/tags/4.12.0`, with
SHA-256 `44c106d5bb47efec04e531fd93008b3fcd1d27138985c5baf4eafac0e1ec9e9d`.
The archive is extracted only under the build tree. CMake verifies its hash
before configuring. No local Homebrew OpenCV or unpinned codec is selected.

The selected source targets are core, imgproc, imgcodecs, features2d, calib3d,
objdetect (which contains ArUco), and calib3d's required flann. All are static.
The bundled third-party build dependencies are zlib 1.3.1, libpng 1.6.43,
and libjpeg-turbo 3.1.0. The 12/16-bit JPEG variants are built internally by
OpenCV's libjpeg-turbo build; the application uses the ordinary JPEG API.
Additional codec libraries, G-API/ADE, Flatbuffers, DNN, video I/O, GUI modules,
Python/Java bindings, GPU backends, IPP, ITT, and nonfree algorithms are disabled.
OpenCV's native macOS imgcodecs sources link the system AppKit framework; it is
a platform runtime, not a fetched third-party component.

## Selected-source license findings

`dependencies/native.json` records the exact archive checksum, source paths,
and notice files. In the selected source, OpenCV's top-level license is
Apache-2.0. Its FLANN module contains BSD-2-Clause code and a BSL-1.0 `any.h`
header. `core/src/softfloat.cpp` carries BSD-3-Clause SoftFloat and Sun FDLIBM
notices. `objdetect/src/aruco/apriltag` credits the University of Michigan and
points to OpenCV's top-level license. Bundled zlib carries the Zlib license;
bundled libpng carries the PNG Reference Library License v2; libjpeg-turbo's
`LICENSE.md` identifies IJG and BSD-3-Clause terms plus Zlib SIMD terms. The
IJG notice requires product documentation to state: "This software is based
in part on the work of the Independent JPEG Group."

These are selected-build metadata and notice checks, not a full source-file
legal audit or a release-artifact audit. The release process must inspect the
actual binaries and include the exact applicable notices. Changes to the
OpenCV version, module list, codec flags, or source archive require a new
inventory review and hash update. The desktop and mobile bundles do not yet
include this backend.
