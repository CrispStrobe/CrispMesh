# Measured stereo reference data

The local measured-data corpus has four default Middlebury pairs and three separately gated ETH3D pairs. All data stays outside version control under `.local-tools/test-data/`. SHA256 and byte lengths are pinned in the [dataset manifests](../tests/datasets/). The downloader verifies existing files without network access by default; downloads require `--fetch`.

For Piano and ETH3D, evaluator inputs are `im0.png`, `im1.png`, `disp0.pfm` or `disp0GT.pfm`, and `calib.txt` in each scene folder. For Cones, Teddy, and Venus, inputs are `im2.png`, `im6.png`, and generated `disp0GT.pfm`; those scenes have no `calib.txt`.

| Scene | Local folder | Image size | Ground truth | Metric calibration |
| --- | --- | ---: | --- | --- |
| Middlebury 2014 Piano-perfect | `middlebury-2014/Piano-perfect` | 2820×1920 | measured PFM disparity | yes, 178.089 mm baseline |
| Middlebury 2003 Cones | `middlebury-2003/cones` | 450×375 | measured PNG disparity, losslessly converted to PFM | no |
| Middlebury 2003 Teddy | `middlebury-2003/teddy` | 450×375 | measured PNG disparity, losslessly converted to PFM | no |
| Middlebury 2001 Venus | `middlebury-2001/venus` | 434×383 | published grayscale disparity, converted to PFM | no |
| ETH3D delivery_area_1s, forest_1s, playground_1s | `eth3d/<scene>` | 711×435, 715×441, 712×436 | laser-scan reference PFM and nonocclusion mask | yes, approximately 60 mm baseline |

For the default corpus:

```sh
python3 scripts/fetch_test_data.py --dataset piano --fetch
python3 scripts/fetch_test_data.py --dataset middlebury2003 --fetch
python3 scripts/fetch_test_data.py --dataset middlebury2001 --fetch
python3 scripts/fetch_test_data.py --dataset piano
python3 scripts/fetch_test_data.py --dataset middlebury2003
python3 scripts/fetch_test_data.py --dataset middlebury2001
CRISPMESH_TEST_REAL_DATA=1 python3 -m unittest scripts/test_fetch_test_data.py
```

The selected ETH3D data requires explicit acknowledgement of its [CC BY-NC-SA 4.0 terms](https://www.eth3d.net/). This local evaluation use does not settle whether a commercial product workflow is permitted. ETH3D is excluded from the default corpus and should not be redistributed with the application:

```sh
python3 scripts/fetch_test_data.py --dataset eth3d --allow-noncommercial-data --fetch
python3 scripts/fetch_test_data.py --dataset eth3d --allow-noncommercial-data
CRISPMESH_TEST_ETH3D=1 python3 -m unittest scripts/test_fetch_test_data.py
```

For another machine, set `CRISPMESH_TEST_DATA_ROOT=/path/to/storage` to move all dataset folders under that base, or pass `--root PATH` for one selected dataset. The fetcher checks that planned writes leave at least 10 GiB free, refuses changed existing files, bounds byte counts, and writes through temporary files in the selected dataset folder. ETH3D archives are checksum verified before extraction; only 15 named members are streamed to pinned paths, so archive paths cannot direct file writes. The `7z` or `7zz` command is needed for ETH3D only. Unit-test scratch files use `.local-tools/tmp/`.

## Source and conversion details

The [Middlebury 2014 collection](https://vision.middlebury.edu/stereo/data/scenes2014/) and [GCPR 2014 paper](https://www.cs.middlebury.edu/~schar/papers/datasets-gcpr2014.pdf) describe Piano's structured-light disparity. The upstream host was unreachable here on 2026-09-26, so the full-resolution four-file pair came from a [pinned third-party mirror commit](https://github.com/ztliu62/stereo3D/tree/91a4aef04c9d46955073281f4555278819b6b589/images/Piano-perfect). Byte equivalence to upstream remains unverified. It is **not** the official quarter-resolution benchmark release. A smaller evaluator result must identify its derived resolution and scaling. Piano's depth relation is `Z_mm = baseline_mm * fx / (disparity_px + doffs_px)`.

The [Middlebury 2003 page](https://vision.middlebury.edu/stereo/data/scenes2003/) says `im2` and `im6` are the rectified two-view pair. `disp2.png` stores quarter-pixel ground truth: gray values 1–255 represent `gray/4` pixels, while zero means unknown. Cones and Teddy source PNGs came from a [pinned mirror commit](https://github.com/beaupreda/semi-global-matching/tree/6643cf4b6a5ab197aace8b766c6ab139e281ae8e). All six files also matched SHA256 from an [independent pinned mirror commit](https://github.com/Soumyabrata/Stereo-Matching/tree/09be6059026e4f831d5360083fcb02afddcccf63/Data). Direct upstream byte comparison remains unavailable. The converter writes bottom-first little-endian PFM with disparity `gray/4` and `+INF` for zero; derived hashes are pinned. No focal length or metric baseline is provided, so these scenes support pixel-disparity assessment only.

The [Middlebury 2001 page](https://vision.middlebury.edu/stereo/data/scenes2001/) defines `im2`, `im6`, `disp2` and a disparity scale of eight. The Venus mirror's PNGs were renamed and may have been re-encoded from the upstream PPM/PGM files; upstream pixel equivalence has not been independently checked. Its converted PFM uses `gray/8` pixels. No zero gray values occur in this mirror map; the converter still reserves zero as unknown. Venus also supports pixel-disparity assessment only.

ETH3D's [benchmark overview](https://www.eth3d.net/overview) describes laser-scanned reference geometry, and its [format documentation](https://www.eth3d.net/documentation) describes PFM disparity and nonocclusion masks. The two official archives are downloaded from `https://www.eth3d.net/data/two_view_training.7z` and `https://www.eth3d.net/data/two_view_training_gt.7z`. The original archives and 15 selected members have independent SHA256 pins. ETH3D's `calib.txt` specifies focal length, baseline, and image dimensions for metric depth checks.

[Middlebury's PFM instructions](https://vision.middlebury.edu/stereo/submit3/upload-format.html) specify that the header scale **sign** selects byte order and its absolute magnitude is ignored. Stored floats are disparities in pixels; rows are bottom-first, and positive infinity marks unknown disparity. The Piano mirror PFM has header scale `-0.003922`, so multiplying its pixel values by `0.003922` would corrupt the reference.

The [Middlebury data page](https://vision.middlebury.edu/stereo/data/) requests citation of the relevant papers when using its datasets. We do not redistribute images or disparity files in this repository. These are two-view stereo tests, not full turntable scanner tests: they lack an ArUco board, rotating object sequence, object masks, and measured finished-object dimensions. See [testing policy](TESTING.md); do not tune thresholds on a scene and then report that same scene as independent acceptance.
