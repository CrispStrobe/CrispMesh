"""Scalar-only, read-only audit of sealed TRAIN two-view geometry row schema.

No feature-index blob, keypoint, camera, pose, image, or mask is selected.
Output is a bounded JSON summary on stdout only.
"""

from __future__ import annotations

from collections import Counter
from contextlib import closing
import hashlib
import json
from pathlib import Path
import shutil
import signal
import sqlite3
import sys


ROOT = Path(__file__).resolve().parents[2]
DATA = Path("/Volumes/backups/code/crispmesh-data")
DATABASE = DATA / "mustard-sfm-masked-fixed-exhaustive-001" / "database.db"
DATABASE_SHA256 = "5858be16dfa3f0fc50303d0eb4a001e21ee14036f67755a9997d5dac374b3075"
MIN_FREE = 10 * 1024**3
MAX_DATABASE_BYTES = 20 * 1024**2
MAX_ROWS = 1_128
MAX_OUTPUT_BYTES = 2 * 1024**2
MAX_SECONDS = 30
MAX_IMAGE_ID = 2_147_483_647


class SchemaAuditError(ValueError):
    pass


def digest(path: Path) -> str:
    result = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def disk_floor() -> dict[str, int]:
    free = {"internal": shutil.disk_usage(ROOT).free, "external": shutil.disk_usage(DATA).free}
    if min(free.values()) < MIN_FREE:
        raise SchemaAuditError("both volumes require at least 10 GiB free")
    return free


def summarize_scalar_rows(rows: list[tuple]) -> dict:
    """Apply exactly the current adapter's geometry-row scalar expectations."""
    if len(rows) > MAX_ROWS:
        raise SchemaAuditError("geometry row count exceeds 48-image maximum")
    configs, row_classes, row_values, columns, lengths = (Counter() for _ in range(5))
    offending = []
    offending_count = 0
    seen_ids = set()
    for pair_id, count, cols, blob_length, config in rows:
        configs[str(config)] += 1
        row_classes[("zero" if count == 0 else "positive" if isinstance(count, int) and count > 0 else "invalid")] += 1
        row_values[str(count)] += 1
        columns[str(cols)] += 1
        if blob_length is None:
            lengths["null"] += 1
        elif isinstance(count, int) and blob_length == count * 8:
            lengths["exact"] += 1
        else:
            lengths["mismatch"] += 1
        reasons = []
        if pair_id in seen_ids:
            reasons.append("duplicate_pair_id")
        seen_ids.add(pair_id)
        if not isinstance(pair_id, int) or pair_id <= 0:
            reasons.append("pair_id")
        else:
            first, second = divmod(pair_id, MAX_IMAGE_ID)
            if not 0 < first < second < MAX_IMAGE_ID:
                reasons.append("pair_id")
        if not isinstance(count, int) or not 0 <= count <= 10_000:
            reasons.append("rows")
        if cols != 2:
            reasons.append("cols")
        if blob_length is None or not isinstance(count, int) or blob_length != count * 8:
            reasons.append("blob_length")
        if not isinstance(config, int) or (isinstance(count, int) and count > 0 and config <= 0):
            reasons.append("config")
        if reasons and len(offending) < 5:
            offending.append({"pair_id": pair_id, "rows": count, "cols": cols,
                              "blob_length": blob_length, "config": config,
                              "reasons": reasons})
        if reasons:
            offending_count += 1
    return {"geometry_rows": len(rows), "config_counts": dict(sorted(configs.items())),
            "row_class_counts": dict(sorted(row_classes.items())),
            "row_value_counts": dict(sorted(row_values.items())),
            "column_counts": dict(sorted(columns.items())),
            "blob_length_validity_counts": dict(sorted(lengths.items())),
            "offending_first_five": offending,
            "offending_count": offending_count}


def audit_database(database: Path, expected_sha256: str) -> dict:
    database = Path(database)
    disk_floor()
    if (database.is_symlink() or not database.is_file() or
            not 0 < database.stat().st_size <= MAX_DATABASE_BYTES or
            digest(database) != expected_sha256):
        raise SchemaAuditError("sealed database path/size/SHA mismatch")
    for suffix in ("-wal", "-shm"):
        sidecar = database.with_name(database.name + suffix)
        if sidecar.exists() or sidecar.is_symlink():
            raise SchemaAuditError("SQLite sidecar present")
    with closing(sqlite3.connect(database.resolve().as_uri() + "?mode=ro&immutable=1", uri=True)) as db:
        db.execute("PRAGMA query_only=ON")
        rows = db.execute("SELECT pair_id,rows,cols,length(data),config "
                          "FROM two_view_geometries ORDER BY pair_id LIMIT ?", (MAX_ROWS + 1,)).fetchall()
    summary = summarize_scalar_rows(rows)
    if digest(database) != expected_sha256:
        raise SchemaAuditError("sealed database changed during audit")
    disk_floor()
    return {"schema": "mustard_geometry_schema_audit_v1", "status": "scalar_only_diagnostic",
            "database_sha256": expected_sha256, "runner_sha256": digest(Path(__file__)),
            "selected_columns": ["pair_id", "rows", "cols", "length(data)", "config"],
            **summary}


def _timeout(_signum, _frame):
    raise TimeoutError("scalar audit exceeded 30 seconds")


def main() -> None:
    if len(sys.argv) != 1:
        raise SchemaAuditError("no arguments or output path accepted")
    if hasattr(signal, "SIGALRM"):
        signal.signal(signal.SIGALRM, _timeout)
        signal.alarm(MAX_SECONDS)
    try:
        payload = (json.dumps(audit_database(DATABASE, DATABASE_SHA256), sort_keys=True,
                              separators=(",", ":")) + "\n").encode()
        if len(payload) > MAX_OUTPUT_BYTES:
            raise SchemaAuditError("scalar report exceeds 2 MiB")
        sys.stdout.buffer.write(payload)
    finally:
        if hasattr(signal, "SIGALRM"):
            signal.alarm(0)


if __name__ == "__main__":
    main()
