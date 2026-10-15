#ifndef CRISPMESH_CORE_H
#define CRISPMESH_CORE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define CRISPMESH_ABI_VERSION 1u
#define CRISPMESH_PROJECT_SCHEMA_VERSION 1u

typedef enum crispmesh_status {
  CRISPMESH_OK = 0,
  CRISPMESH_INVALID_ARGUMENT = 1,
  CRISPMESH_INVALID_PROJECT = 2,
  CRISPMESH_BUFFER_TOO_SMALL = 3,
  CRISPMESH_UNAVAILABLE = 4,
  CRISPMESH_INTERNAL_ERROR = 5
} crispmesh_status;

/* All JSON results are UTF-8 and NUL-terminated. `required` includes the NUL.
 * Pass output=NULL, capacity=0 to query the required size. This ABI neither
 * owns caller memory nor retains input pointers. */
uint32_t crispmesh_abi_version(void);
crispmesh_status crispmesh_capabilities_json(char *output, size_t capacity,
                                           size_t *required);
crispmesh_status crispmesh_validate_project_json(const char *input, size_t length,
                                                char *output, size_t capacity,
                                                size_t *required);
crispmesh_status crispmesh_geometry_diagnostic_json(char *output, size_t capacity,
                                                   size_t *required);
crispmesh_status crispmesh_reconstruct_json(const char *input, size_t length,
                                          char *output, size_t capacity,
                                          size_t *required);
/* Reads project_path and its referenced PNG/JPEG images. Result matrices are
 * row-major object-to-camera transforms; translations use millimetres.
 * Caller owns all buffers. No source file is modified. */
crispmesh_status crispmesh_estimate_poses_json(const char *project_path,
                                             char *output, size_t capacity,
                                             size_t *required);
/* Computes once and returns a heap-owned NUL-terminated report. Release with
 * crispmesh_free; size includes the NUL. This avoids executing image work twice
 * when the caller cannot preallocate a result buffer. */
crispmesh_status crispmesh_estimate_poses_alloc(const char *project_path,
                                              char **output, size_t *size);
/* Re-estimates all image poses, then returns a masked sparse point report.
 * Available only in an OpenCV-enabled build; release output with crispmesh_free. */
crispmesh_status crispmesh_reconstruct_sparse_alloc(const char *project_path,
                                                  char **output, size_t *size);
void crispmesh_free(void *pointer);

#ifdef __cplusplus
}
#endif

#endif
