cmake_minimum_required(VERSION 3.20)
project(crispmesh_selected_mve LANGUAGES CXX)

# This is a selected-target build of the pinned MVE source after sift_only.patch.
# It intentionally never builds upstream's GPL sceneupgrade or SURF source.
set(MVE_SOURCE "" CACHE PATH "Pinned MVE source root after sift_only.patch")
if(NOT EXISTS "${MVE_SOURCE}/LICENSE.txt")
  message(FATAL_ERROR "Set MVE_SOURCE to the pinned, patched MVE source directory")
endif()
get_filename_component(REPO_ROOT "${CMAKE_CURRENT_LIST_DIR}/../.." ABSOLUTE)
set(MVE_ARCHIVE "${REPO_ROOT}/.local-tools/mve-spike/mve-bf2279f.tar.gz"
    CACHE FILEPATH "Pinned MVE archive required for exact selected-source verification")
find_package(Python3 COMPONENTS Interpreter REQUIRED)
execute_process(
  COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_LIST_DIR}/verify_source.py"
          --archive "${MVE_ARCHIVE}" --source "${MVE_SOURCE}"
  RESULT_VARIABLE VERIFY_CODE OUTPUT_VARIABLE VERIFY_OUTPUT ERROR_VARIABLE VERIFY_ERROR)
if(NOT VERIFY_CODE EQUAL 0)
  message(FATAL_ERROR "Selected source verification failed: ${VERIFY_OUTPUT} ${VERIFY_ERROR}")
endif()
message(STATUS "${VERIFY_OUTPUT}")
file(READ "${MVE_SOURCE}/apps/sfmrecon/sfmrecon.cc" SFM_APP_SOURCE)
if(NOT SFM_APP_SOURCE MATCHES "feature_types = sfm::FeatureSet::FEATURE_SIFT;")
  message(FATAL_ERROR "SIFT-only source patch is missing")
endif()
file(READ "${MVE_SOURCE}/libs/sfm/feature_set.cc" FEATURES_SOURCE)
if(FEATURES_SOURCE MATCHES "this->compute_surf\\(image\\)")
  message(FATAL_ERROR "SURF call remains in selected source")
endif()

# macOS can have an old Mono.framework header alongside a newer Unix-style
# dylib (e.g. png 1.4 headers and png 1.6 library). CMake caches find_path
# results, so changing the search preference alone does not repair an existing
# build directory. Discard only that demonstrably mixed combination.
if(APPLE)
  set(CMAKE_FIND_FRAMEWORK NEVER)
  foreach(codec IN ITEMS JPEG PNG TIFF)
    if(codec STREQUAL "PNG")
      set(header_cache PNG_PNG_INCLUDE_DIR)
    else()
      set(header_cache ${codec}_INCLUDE_DIR)
    endif()
    if(DEFINED ${header_cache} AND
       "${${header_cache}}" MATCHES "\\.framework(/|$)" AND
       DEFINED ${codec}_LIBRARY_RELEASE AND
       NOT "${${codec}_LIBRARY_RELEASE}" MATCHES "\\.framework(/|$)")
      message(STATUS "Discarding mixed ${codec} framework header cache: ${${header_cache}}")
      unset(${header_cache} CACHE)
    endif()
  endforeach()
endif()

find_package(JPEG REQUIRED)
find_package(PNG REQUIRED)
find_package(TIFF REQUIRED)
find_package(Threads REQUIRED)
foreach(codec IN ITEMS JPEG PNG TIFF)
  if(codec STREQUAL "PNG")
    set(header_cache PNG_PNG_INCLUDE_DIR)
  else()
    set(header_cache ${codec}_INCLUDE_DIR)
  endif()
  message(STATUS "${codec} headers: ${${header_cache}}; library: ${${codec}_LIBRARY_RELEASE}")
  if(APPLE AND "${${header_cache}}" MATCHES "\\.framework(/|$)" AND
     NOT "${${codec}_LIBRARY_RELEASE}" MATCHES "\\.framework(/|$)")
    message(FATAL_ERROR "${codec} framework headers and non-framework library are mixed")
  endif()
endforeach()
set(MVE_IMAGE_LIBS JPEG::JPEG PNG::PNG TIFF::TIFF Threads::Threads)
option(CRISPMESH_MVE_OPENMP "Enable OpenMP for selected CPU tools (unmeasured)" OFF)
if(CRISPMESH_MVE_OPENMP)
  find_package(OpenMP REQUIRED COMPONENTS CXX)
  list(APPEND MVE_IMAGE_LIBS OpenMP::OpenMP_CXX)
endif()

function(selected_sources output folder)
  file(GLOB candidates CONFIGURE_DEPENDS "${MVE_SOURCE}/libs/${folder}/[^_]*.cc")
  if(folder STREQUAL "sfm")
    list(FILTER candidates EXCLUDE REGEX "/surf\\.cc$")
  endif()
  set(${output} "${candidates}" PARENT_SCOPE)
endfunction()

foreach(part IN ITEMS util mve sfm dmrecon fssr)
  selected_sources(sources "${part}")
  add_library(mve_${part} STATIC ${sources})
  target_compile_features(mve_${part} PUBLIC cxx_std_17)
  target_include_directories(mve_${part} PUBLIC "${MVE_SOURCE}/libs")
  target_link_libraries(mve_${part} PUBLIC ${MVE_IMAGE_LIBS})
  if(WIN32)
    # windows.h min/max macros corrupt std::numeric_limits and MVE's templates.
    # PUBLIC carries the definition into selected application translation units.
    target_compile_definitions(mve_${part} PUBLIC NOMINMAX)
  endif()
endforeach()
target_link_libraries(mve_mve PUBLIC mve_util)
target_link_libraries(mve_sfm PUBLIC mve_mve)
target_link_libraries(mve_dmrecon PUBLIC mve_mve)
target_link_libraries(mve_fssr PUBLIC mve_mve)

function(selected_app name)
  file(GLOB sources CONFIGURE_DEPENDS "${MVE_SOURCE}/apps/${name}/[^_]*.cc")
  add_executable(${name} ${sources})
  target_include_directories(${name} PRIVATE "${MVE_SOURCE}/libs")
  target_compile_features(${name} PRIVATE cxx_std_17)
  if(name STREQUAL "sfmrecon")
    target_link_libraries(${name} PRIVATE mve_sfm)
  elseif(name STREQUAL "dmrecon")
    target_link_libraries(${name} PRIVATE mve_dmrecon)
  elseif(name STREQUAL "fssrecon" OR name STREQUAL "meshclean")
    target_link_libraries(${name} PRIVATE mve_fssr)
  else()
    target_link_libraries(${name} PRIVATE mve_mve)
  endif()
endfunction()

foreach(name IN ITEMS makescene sfmrecon dmrecon scene2pset fssrecon meshclean)
  selected_app(${name})
endforeach()

enable_testing()
add_executable(selected_mve_png_abi_probe codec_probe.cpp)
target_link_libraries(selected_mve_png_abi_probe PRIVATE PNG::PNG)
add_test(NAME selected_mve_png_abi COMMAND selected_mve_png_abi_probe)
add_test(NAME selected_mve_cli_smoke
  COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_LIST_DIR}/check_binaries.py"
          $<TARGET_FILE:makescene> $<TARGET_FILE:sfmrecon> $<TARGET_FILE:dmrecon>
          $<TARGET_FILE:scene2pset> $<TARGET_FILE:fssrecon> $<TARGET_FILE:meshclean>)
