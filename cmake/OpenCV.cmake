# Opt-in, source-pinned OpenCV for calibrated image geometry. Include from the
# top-level CMakeLists.txt after project(); link ${CRISPMESH_OPENCV_LIBRARIES}.
option(CRISPMESH_WITH_OPENCV "Build pinned OpenCV image geometry support" OFF)

if(CRISPMESH_WITH_OPENCV)
  include(FetchContent)

  # Keep this set deliberately narrow. In particular, require the bundled
  # codecs so that a host's unrelated libraries cannot change the build.
  set(BUILD_LIST "core,imgproc,imgcodecs,features2d,calib3d,objdetect" CACHE STRING "OpenCV modules" FORCE)
  set(BUILD_SHARED_LIBS OFF CACHE BOOL "Static OpenCV libraries" FORCE)
  set(BUILD_TESTS OFF CACHE BOOL "OpenCV tests" FORCE)
  set(BUILD_PERF_TESTS OFF CACHE BOOL "OpenCV performance tests" FORCE)
  set(BUILD_EXAMPLES OFF CACHE BOOL "OpenCV examples" FORCE)
  set(BUILD_opencv_apps OFF CACHE BOOL "OpenCV applications" FORCE)
  set(BUILD_JAVA OFF CACHE BOOL "OpenCV Java" FORCE)
  set(BUILD_opencv_python2 OFF CACHE BOOL "OpenCV Python 2" FORCE)
  set(BUILD_opencv_python3 OFF CACHE BOOL "OpenCV Python 3" FORCE)
  set(BUILD_ZLIB ON CACHE BOOL "Bundled zlib" FORCE)
  set(BUILD_PNG ON CACHE BOOL "Bundled libpng" FORCE)
  set(BUILD_JPEG ON CACHE BOOL "Bundled libjpeg-turbo" FORCE)
  set(BUILD_JPEG_TURBO_DISABLE OFF CACHE BOOL "Use libjpeg-turbo" FORCE)
  set(WITH_PNG ON CACHE BOOL "PNG decoding" FORCE)
  set(WITH_JPEG ON CACHE BOOL "JPEG decoding" FORCE)

  foreach(_feature 1394 ADE AVFOUNDATION AVIF CAROTENE CUDA EIGEN FFMPEG FLATBUFFERS GDCM GSTREAMER
                   GTK HALIDE HDF5 IMGCODEC_GIF IMGCODEC_HDR IMGCODEC_SUNRASTER
                   IMGCODEC_PXM IMGCODEC_PFM IPP ITT JASPER JPEGXL KLEIDICV LAPACK MFX
                   OBSENSOR OPENCL OPENCLAMDBLAS OPENCLAMDFFT OPENEXR OPENGL OPENJPEG OPENMP
                   PROTOBUF QT QUIRC SPNG TBB TIFF V4L VA VTK WEBP)
    set(WITH_${_feature} OFF CACHE BOOL "Excluded from CrispMesh OpenCV build" FORCE)
  endforeach()
  set(OPENCV_ENABLE_NONFREE OFF CACHE BOOL "Exclude nonfree algorithms" FORCE)
  set(OPENCV_GENERATE_PKGCONFIG OFF CACHE BOOL "No OpenCV pkg-config export" FORCE)
  set(OPENCV_DOWNLOAD_PATH "${CMAKE_BINARY_DIR}/opencv-downloads" CACHE PATH "OpenCV download cache" FORCE)

  # DOWNLOAD_EXTRACT_TIMESTAMP was added in CMake 3.24. Earlier supported
  # CMake versions can still fetch the hash-pinned archive without it.
  set(_crispmesh_opencv_timestamp_arg)
  if(CMAKE_VERSION VERSION_GREATER_EQUAL 3.24)
    set(_crispmesh_opencv_timestamp_arg DOWNLOAD_EXTRACT_TIMESTAMP FALSE)
  endif()
  FetchContent_Declare(crispmesh_opencv
    URL "https://codeload.github.com/opencv/opencv/tar.gz/refs/tags/4.12.0"
    URL_HASH "SHA256=44c106d5bb47efec04e531fd93008b3fcd1d27138985c5baf4eafac0e1ec9e9d"
    ${_crispmesh_opencv_timestamp_arg}
  )
  FetchContent_MakeAvailable(crispmesh_opencv)

  set(CRISPMESH_OPENCV_LIBRARIES
      opencv_core opencv_imgproc opencv_imgcodecs
      opencv_flann opencv_features2d opencv_calib3d opencv_objdetect)
  set(CRISPMESH_OPENCV_INCLUDE_DIRS "${CMAKE_BINARY_DIR}")
  foreach(_module core imgproc imgcodecs flann features2d calib3d objdetect)
    list(APPEND CRISPMESH_OPENCV_INCLUDE_DIRS
      "${crispmesh_opencv_SOURCE_DIR}/modules/${_module}/include")
  endforeach()
  foreach(_target IN LISTS CRISPMESH_OPENCV_LIBRARIES)
    if(NOT TARGET ${_target})
      message(FATAL_ERROR "Required OpenCV target ${_target} was not built")
    endif()
  endforeach()
endif()
