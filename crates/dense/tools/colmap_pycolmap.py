"""A stand-in for the `colmap` executable built on the PyCOLMAP wheel.

Development aid for the `colmap` camera provider of `crispmesh-dense photos`
(crates/dense/src/photos/cameras.rs) on machines without a COLMAP executable:
it accepts the command lines the provider builds and runs the same COLMAP
library steps through PyCOLMAP.

    crispmesh-dense photos ... --cameras colmap --colmap crates/dense/tools/colmap_pycolmap.py --python <python with pycolmap>

Covered: `feature_extractor`, `exhaustive_matcher`, `sequential_matcher`,
`mapper`, with options written as `--Group.name value` (COLMAP 3 names). Not
covered: `matches_importer` (PyCOLMAP 3.11 has no call for matching a list of
pairs), so `--colmap-matching ring` needs the real executable. Unknown options
are an error, as they are for COLMAP. Written against PyCOLMAP 3.11.
"""

import sys

import pycolmap


def parse(arguments):
    options, index = {}, 0
    while index < len(arguments):
        flag = arguments[index]
        if not flag.startswith("--") or index + 1 >= len(arguments):
            raise SystemExit(f"colmap_pycolmap: expected --name value, found {flag!r}")
        options[flag[2:]] = arguments[index + 1]
        index += 2
    return options


def assign(target, name, text):
    """Set an option from its command-line text, converted to the type the option already has."""
    if not hasattr(target, name):
        raise SystemExit(f"colmap_pycolmap: unknown option {type(target).__name__}.{name}")
    current = getattr(target, name)
    if isinstance(current, bool):
        value = text.lower() in ("1", "true", "yes")
    elif isinstance(current, int):
        value = int(text)
    elif isinstance(current, float):
        value = float(text)
    else:
        value = text
    setattr(target, name, value)


def take(options, group, target, skip=()):
    for key in [k for k in options if k.startswith(group + ".")]:
        name = key.split(".", 1)[1]
        text = options.pop(key)
        if name not in skip:
            assign(target, name, text)


def main(argv):
    if len(argv) < 2:
        raise SystemExit(__doc__)
    command, options = argv[1], parse(argv[2:])
    database = options.pop("database_path")
    if command == "feature_extractor":
        reader, sift = pycolmap.ImageReaderOptions(), pycolmap.SiftExtractionOptions()
        single = options.pop("ImageReader.single_camera", "0") == "1"
        model = options.pop("ImageReader.camera_model", "SIMPLE_RADIAL")
        take(options, "ImageReader", reader)
        gpu = options.pop("SiftExtraction.use_gpu", "0") == "1"
        take(options, "SiftExtraction", sift)
        images = options.pop("image_path")
        if options:
            raise SystemExit(f"colmap_pycolmap: unknown options {sorted(options)}")
        pycolmap.extract_features(database, images, camera_mode=pycolmap.CameraMode.SINGLE if single else pycolmap.CameraMode.AUTO,
                                  camera_model=model, reader_options=reader, sift_options=sift,
                                  device=pycolmap.Device.auto if gpu else pycolmap.Device.cpu)
    elif command in ("exhaustive_matcher", "sequential_matcher"):
        sift = pycolmap.SiftMatchingOptions()
        gpu = options.pop("SiftMatching.use_gpu", "0") == "1"
        take(options, "SiftMatching", sift)
        device = pycolmap.Device.auto if gpu else pycolmap.Device.cpu
        if command == "exhaustive_matcher":
            matching = pycolmap.ExhaustiveMatchingOptions()
            take(options, "ExhaustiveMatching", matching)
            call = pycolmap.match_exhaustive
        else:
            matching = pycolmap.SequentialMatchingOptions()
            take(options, "SequentialMatching", matching)
            call = pycolmap.match_sequential
        if options:
            raise SystemExit(f"colmap_pycolmap: unknown options {sorted(options)}")
        call(database, sift_options=sift, matching_options=matching, device=device)
    elif command == "mapper":
        pipeline = pycolmap.IncrementalPipelineOptions()
        images, output = options.pop("image_path"), options.pop("output_path")
        for key in [k for k in options if k.startswith("Mapper.")]:
            name, text = key.split(".", 1)[1], options.pop(key)
            # COLMAP's command line puts the pipeline's and the inner mapper's options in one group.
            assign(pipeline if hasattr(pipeline, name) else pipeline.mapper, name, text)
        if options:
            raise SystemExit(f"colmap_pycolmap: unknown options {sorted(options)}")
        models = pycolmap.incremental_mapping(database, images, output, options=pipeline)
        for index, model in models.items():
            print(f"model {index}: {model.num_reg_images()} images, {model.num_points3D()} points")
        if not models:
            raise SystemExit("colmap_pycolmap: the mapper found no model")
    else:
        raise SystemExit(f"colmap_pycolmap: command {command!r} is not covered by this stand-in (see its docstring)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
