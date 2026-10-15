/* C interface of the crispmesh-dense run driver (crates/dense, feature "capi").
 *
 * Build the library:
 *   cargo rustc --release --lib --features capi --crate-type cdylib      (shared)
 *   cargo rustc --release --lib --features capi --crate-type staticlib   (static)
 *
 * A run executes on a thread of its own. Start it with a JSON object of options,
 * poll for events and the result, cancel if needed, free the handle.
 *
 * Options (all but "output" and a starting point are optional):
 *   {
 *     "output": "/path/to/fresh/run/directory",
 *     "inputs": "/path/to/inputs",                 // or "scene", "prepared" and "raw_masks",
 *                                                  // or "photos": "/path/to/photos" with
 *     "photo_options": ["--calibration", "lens.json", "--masks", "threshold"],
 *                                                  // (words of `crispmesh-dense photos`)
 *     "config": "/path/to/config.json",
 *     "preset": "detail",                         // optional named settings, see describe()
 *     "overrides": ["grid=320"],
 *     "settings": {"sizes": [256, 512]},           // names as in the settings list
 *     "threads": 2, "stereo_timeout": 3600, "minimum_free_gib": 2.0,
 *     "reuse_depths": null, "live_previews": true, "preview_step": 2,
 *     "check": true, "preview": true, "keep_volume": false
 *   }
 *
 * Poll result:
 *   {"finished": false, "status": "running", "events": [...], "report": null, "error": null}
 * "events" holds the events emitted since the previous poll, in the format of
 * docs/ENGINE-CONTRACT.md (the same lines go to <output>/events.jsonl). When
 * "finished" is true, "status" is "complete", "failed" or "cancelled", "report"
 * is the content of pipeline.json on success and "error" a message otherwise.
 *
 * Every char* returned by the library is UTF-8, NUL-terminated and must be
 * released with crispmesh_string_free, except the version string.
 */
#ifndef CRISPMESH_DENSE_H
#define CRISPMESH_DENSE_H

#ifdef __cplusplus
extern "C" {
#endif

typedef struct CrispMeshRun CrispMeshRun;

/* Version of the library; static, do not free. */
const char *crispmesh_version(void);

/* Starts a run. Returns NULL when the options are invalid; if error is not
 * NULL, *error then receives a message to free with crispmesh_string_free. */
CrispMeshRun *crispmesh_run_start(const char *options_json, char **error);

/* New events and the state of the run as a JSON string (see above). */
char *crispmesh_run_poll(CrispMeshRun *run);

/* Asks the run to stop. It ends with status "cancelled" at the next view while
 * matching, or at the next stage boundary. */
void crispmesh_run_cancel(CrispMeshRun *run);

/* Cancels the run if it is still going, waits for it and releases the handle. */
void crispmesh_run_free(CrispMeshRun *run);

/* Releases a string returned by this library. NULL is accepted. */
void crispmesh_string_free(char *text);

#ifdef __cplusplus
}
#endif

#endif
