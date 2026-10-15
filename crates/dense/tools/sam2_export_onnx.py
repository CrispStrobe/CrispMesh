"""Export SAM 2.1 (image mode) to the two ONNX graphs the `sam` mask provider runs.

A build-time tool: the crate itself needs no Python. From a SAM 2 source
checkout and a checkpoint it writes into a fresh directory

    encoder.onnx   image [1,3,1024,1024] float32, already resized and normalised
                   -> image_embed [1,256,64,64], high_res_0 [1,32,256,256], high_res_1 [1,64,128,128]
    decoder.onnx   the three encoder outputs, point_coords [1,N,2] float32 (pixels of the
                   1024 frame; box corners first), point_labels [1,N] int64 (1 object,
                   0 background, 2 and 3 the two box corners)
                   -> mask_logits [1,4,256,256], iou [1,4] (token 0: single mask, 1..3: multimask)
    model.json     sizes, normalisation, tensor names, SHA-256 of every file, license

Everything outside the network (resizing, prompts, choosing among the masks,
upsampling, cleanup) is done by the caller; `--check DIR` compares the graphs
with PyTorch on real photos, doing those steps the way the provider does.

    python sam2_export_onnx.py --source SAM2_CHECKOUT --checkpoint sam2.1_hiera_tiny.pt --output DIR \
        [--site-packages DIR] [--check WORK_DIR --check-views 8] [--variants fp16,int8]

Needs torch, onnx and (for --check and the variants) onnxruntime. The memory
parts of SAM 2 (video) are not exported.
"""

import argparse
import hashlib
import json
from pathlib import Path
import sys

MODEL_CONFIG = "configs/sam2.1/sam2.1_hiera_t.yaml"
SCHEMA = "crispmesh_sam_model_v1"
LICENSE = "Apache-2.0 (SAM 2, Copyright Meta Platforms, Inc. and affiliates)"
MEAN = [0.485, 0.456, 0.406]
STD = [0.229, 0.224, 0.225]


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def build(source, checkpoint, config):
    import torch

    sys.path.insert(0, str(source.resolve()))
    try:
        from sam2.build_sam import build_sam2

        model = build_sam2(config, str(checkpoint), device="cpu", apply_postprocessing=False)
    finally:
        sys.path.pop(0)
    model.eval()
    torch.set_grad_enabled(False)
    return model


def wrappers(model):
    import torch
    from torch import nn
    import torch.nn.functional as F

    trunk = model.image_encoder.trunk
    size = model.image_size
    # The trunk interpolates its small position embedding (bicubic) to the token
    # grid and adds a tiled window embedding at every call. Bicubic resampling
    # is separable and linear, so with a fixed input it is two products with a
    # constant matrix: no Resize operator, and no 25 MB constant in the file.
    grid = size // 4
    frozen = trunk._get_pos_embed((grid, grid)).detach().clone()
    small = trunk.pos_embed.detach()[0]
    side = small.shape[-1]
    weights = F.interpolate(torch.eye(side)[None, None], size=(grid, side), mode="bicubic")[0, 0]
    window = trunk.pos_embed_window.detach()[0]
    repeat = grid // window.shape[-1]

    class Encoder(nn.Module):
        def __init__(self):
            super().__init__()
            self.model = model
            self.register_buffer("pos_small", small.clone())
            self.register_buffer("pos_weights", weights.clone())
            self.register_buffer("pos_window", window.permute(1, 2, 0).clone())

        def position(self, hw):
            wide = torch.matmul(self.pos_weights, torch.matmul(self.pos_small, self.pos_weights.t()))
            return wide.permute(1, 2, 0).unsqueeze(0) + self.pos_window.repeat(repeat, repeat, 1)

        def forward(self, image):
            out = self.model.forward_image(image)
            _, feats, _, sizes = self.model._prepare_backbone_features(out)
            if self.model.directly_add_no_mem_embed:
                feats[-1] = feats[-1] + self.model.no_mem_embed
            maps = [f.permute(1, 2, 0).reshape(1, -1, s[0], s[1]) for f, s in zip(feats, sizes)]
            return maps[2], maps[0], maps[1]

    class Decoder(nn.Module):
        def __init__(self):
            super().__init__()
            self.prompt = model.sam_prompt_encoder
            self.decoder = model.sam_mask_decoder
            self.register_buffer("dense_pe", self.prompt.get_dense_pe().detach().clone())

        def forward(self, image_embed, high_res_0, high_res_1, point_coords, point_labels):
            sparse, dense = self.prompt(points=(point_coords, point_labels), boxes=None, masks=None)
            masks, iou, _, _ = self.decoder.predict_masks(
                image_embeddings=image_embed,
                image_pe=self.dense_pe,
                sparse_prompt_embeddings=sparse,
                dense_prompt_embeddings=dense,
                repeat_image=False,
                high_res_features=[high_res_0, high_res_1],
            )
            return masks, iou

    encoder = Encoder().eval()
    difference = float((encoder.position((grid, grid)) - frozen).abs().max())
    if difference > 1e-5:
        raise ValueError(f"position embedding as matrix products differs from the trunk's by {difference}")
    trunk._get_pos_embed = encoder.position
    return encoder, Decoder().eval()


def export(model, output, opset):
    import torch

    encoder, decoder = wrappers(model)
    size = model.image_size
    image = torch.zeros(1, 3, size, size)
    torch.onnx.export(
        encoder,
        (image,),
        str(output / "encoder.onnx"),
        opset_version=opset,
        input_names=["image"],
        output_names=["image_embed", "high_res_0", "high_res_1"],
        do_constant_folding=True,
        dynamo=False,
    )
    embed, high0, high1 = encoder(image)
    coords = torch.tensor([[[100.0, 120.0], [900.0, 800.0], [500.0, 400.0], [10.0, 20.0]]])
    labels = torch.tensor([[2, 3, 1, 0]], dtype=torch.int64)
    torch.onnx.export(
        decoder,
        (embed, high0, high1, coords, labels),
        str(output / "decoder.onnx"),
        opset_version=opset,
        input_names=["image_embed", "high_res_0", "high_res_1", "point_coords", "point_labels"],
        output_names=["mask_logits", "iou"],
        dynamic_axes={"point_coords": {1: "points"}, "point_labels": {1: "points"}},
        do_constant_folding=True,
        dynamo=False,
    )
    return encoder, decoder


def operators(path):
    import onnx

    counts = {}
    for node in onnx.load(str(path)).graph.node:
        counts[node.op_type] = counts.get(node.op_type, 0) + 1
    return dict(sorted(counts.items()))


def variants(output, which):
    """Smaller files for the size and accuracy comparison; the provider's default is float32."""
    written = {}
    if "fp16" in which:
        import onnx
        from onnxconverter_common import float16

        for name in ("encoder", "decoder"):
            graph = float16.convert_float_to_float16(onnx.load(str(output / f"{name}.onnx")), keep_io_types=True)
            onnx.save(graph, str(output / f"{name}.fp16.onnx"))
            written[f"{name}.fp16.onnx"] = None
    if "int8" in which:
        from onnxruntime.quantization import QuantType, quantize_dynamic

        for name in ("encoder", "decoder"):
            quantize_dynamic(
                str(output / f"{name}.onnx"),
                str(output / f"{name}.int8.onnx"),
                weight_type=QuantType.QUInt8,
                op_types_to_quantize=["MatMul", "Gemm"],
            )
            written[f"{name}.int8.onnx"] = None
    return list(written)


# ---- the steps around the network, as the provider does them (and as SAM2ImagePredictor does) ----


def prepare(rgb, size):
    """ToTensor, antialiased bilinear resize to size x size, normalisation: SAM2Transforms."""
    import torch
    import torch.nn.functional as F

    tensor = torch.from_numpy(rgb).permute(2, 0, 1).float().div(255.0)[None]
    tensor = F.interpolate(tensor, size=(size, size), mode="bilinear", align_corners=False, antialias=True)
    mean = torch.tensor(MEAN).view(1, 3, 1, 1)
    std = torch.tensor(STD).view(1, 3, 1, 1)
    return ((tensor - mean) / std).numpy()


def model_prompts(prompt, width, height, size):
    """Box corners first, then the points, in the pixels of the model frame."""
    import numpy as np

    box = np.asarray(prompt["box_xyxy"], np.float32).reshape(2, 2)
    points = np.asarray(prompt.get("points_xy", [prompt["point_xy"]]), np.float32)
    coords = np.concatenate([box, points]) / np.array([width, height], np.float32) * size
    labels = np.concatenate([[2, 3], prompt.get("point_labels", [1])]).astype(np.int64)
    return coords[None].astype(np.float32), labels[None]


def full_masks(logits, width, height):
    import torch
    import torch.nn.functional as F

    up = F.interpolate(torch.from_numpy(logits), (height, width), mode="bilinear", align_corners=False)
    return (up > 0.0).numpy()[0]


def check(model, output, work, views, files, report):
    """PyTorch predictor against the exported graphs on photos of a `photos --stop-after masks` run."""
    import numpy as np
    import onnxruntime as ort
    from PIL import Image
    import torch

    from sam2.sam2_image_predictor import SAM2ImagePredictor
    from scripts.turntable_mesh.segment import frozen_prompts, select_prediction, selected_images

    paths = selected_images(work / "photos", 0)
    prompts, (width, height) = frozen_prompts(paths, work / "coarse-masks", automatic_cues=True)
    chosen = np.linspace(0, len(paths), views, endpoint=False, dtype=int) if views < len(paths) else range(len(paths))
    predictor = SAM2ImagePredictor(model)
    size = model.image_size
    options = ort.SessionOptions()
    options.intra_op_num_threads = 4
    rows = {}
    for encoder_file, decoder_file in files:
        encoder = ort.InferenceSession(str(output / encoder_file), options, providers=["CPUExecutionProvider"])
        decoder = ort.InferenceSession(str(output / decoder_file), options, providers=["CPUExecutionProvider"])
        row = {"embed_abs": [], "logit_abs": [], "iou_abs": [], "mask_iou_selected": [], "mask_iou_all": [], "selected_same": []}
        for index in chosen:
            path, prompt = paths[index], prompts[index]
            rgb = np.array(Image.open(path).convert("RGB"))
            predictor.set_image(rgb)
            reference = predictor._features
            masks, scores, low = predictor.predict(
                box=np.asarray(prompt["box_xyxy"], np.float32),
                point_coords=np.asarray(prompt["points_xy"], np.float32),
                point_labels=np.asarray(prompt["point_labels"], np.int32),
                multimask_output=True,
            )
            _, clean, metrics = select_prediction(masks, scores, prompt, (width, height), preserve_holes=True)
            embed, high0, high1 = encoder.run(None, {"image": prepare(rgb, size)})
            row["embed_abs"].append(float(np.abs(embed - reference["image_embed"].numpy()).max()))
            coords, labels = model_prompts(prompt, width, height, size)
            logits, iou = decoder.run(
                None,
                {"image_embed": embed, "high_res_0": high0, "high_res_1": high1, "point_coords": coords, "point_labels": labels},
            )
            row["logit_abs"].append(float(np.abs(np.clip(logits[0, 1:], -32, 32) - low).max()))
            row["iou_abs"].append(float(np.abs(iou[0, 1:] - scores).max()))
            ours = full_masks(logits[:, 1:], width, height)
            both = [(a & (b > 0)).sum() / max(1, (a | (b > 0)).sum()) for a, b in zip(ours, masks)]
            row["mask_iou_all"].append(float(min(both)))
            _, ours_clean, ours_metrics = select_prediction(ours, iou[0, 1:], prompt, (width, height), preserve_holes=True)
            row["selected_same"].append(ours_metrics["selected_index"] == metrics["selected_index"])
            row["mask_iou_selected"].append(float((ours_clean & clean).sum() / max(1, (ours_clean | clean).sum())))
        rows[encoder_file] = {
            "views": len(row["embed_abs"]),
            "image_embed_max_abs_difference": max(row["embed_abs"]),
            "mask_logit_max_abs_difference": max(row["logit_abs"]),
            "predicted_iou_max_abs_difference": max(row["iou_abs"]),
            "raw_mask_iou_minimum_over_candidates": min(row["mask_iou_all"]),
            "selected_mask_iou_median": float(np.median(row["mask_iou_selected"])),
            "selected_mask_iou_minimum": min(row["mask_iou_selected"]),
            "same_candidate_selected": int(sum(row["selected_same"])),
        }
        print(encoder_file, json.dumps(rows[encoder_file]))
    report["check"] = {"work": str(work), "photo_size": [width, height], "torch": torch.__version__, "onnxruntime": ort.__version__, "graphs": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--source", type=Path, required=True, help="SAM 2 source checkout")
    parser.add_argument("--checkpoint", type=Path, required=True)
    parser.add_argument("--config", default=MODEL_CONFIG, help="relative to source/sam2")
    parser.add_argument("--output", type=Path, required=True, help="fresh directory")
    parser.add_argument("--opset", type=int, default=17)
    parser.add_argument("--site-packages", type=Path, action="append", default=[], help="extra module directory (onnx, onnxruntime)")
    parser.add_argument("--repository", type=Path, help="checkout with scripts/turntable_mesh (for --check)")
    parser.add_argument("--variants", default="", help="comma-separated: fp16, int8")
    parser.add_argument("--check", type=Path, help="work directory of `crispmesh-dense photos --stop-after masks` (photos/, coarse-masks/)")
    parser.add_argument("--check-views", type=int, default=8)
    args = parser.parse_args()
    for folder in args.site_packages + ([args.repository] if args.repository else []):
        sys.path.append(str(folder.resolve()))
    args.output.mkdir(parents=True)
    model = build(args.source, args.checkpoint, args.config)
    export(model, args.output, args.opset)
    extra = variants(args.output, [v for v in args.variants.split(",") if v])
    files = ["encoder.onnx", "decoder.onnx"] + extra
    import onnx
    import torch

    report = {
        "schema": SCHEMA,
        "model": "sam2.1_hiera_tiny" if args.config == MODEL_CONFIG else args.config,
        "license": LICENSE,
        "format": "onnx",
        "opset": args.opset,
        "image_size": model.image_size,
        "mask_size": model.image_size // 4,
        "mean": MEAN,
        "std": STD,
        "encoder": "encoder.onnx",
        "decoder": "decoder.onnx",
        "dynamic_multimask": {
            "enabled": bool(model.sam_mask_decoder.dynamic_multimask_via_stability),
            "stability_delta": float(model.sam_mask_decoder.dynamic_multimask_stability_delta),
            "stability_threshold": float(model.sam_mask_decoder.dynamic_multimask_stability_thresh),
        },
        "files": {name: {"bytes": (args.output / name).stat().st_size, "sha256": sha256(args.output / name)} for name in files},
        "operators": {name: operators(args.output / name) for name in ("encoder.onnx", "decoder.onnx")},
        "exported_from": {"checkpoint_sha256": sha256(args.checkpoint), "config": args.config, "torch": torch.__version__, "onnx": onnx.__version__},
    }
    if args.check:
        pairs = [("encoder.onnx", "decoder.onnx")] + [
            (f"encoder.{v}.onnx", f"decoder.{v}.onnx") for v in ("fp16", "int8") if f"encoder.{v}.onnx" in extra
        ]
        sys.path.insert(0, str(args.source.resolve()))
        check(model, args.output, args.check, args.check_views, pairs, report)
    (args.output / "model.json").write_text(json.dumps(report, indent=1) + "\n")
    print(json.dumps({"output": str(args.output.resolve()), "files": report["files"]}, indent=1))


if __name__ == "__main__":
    main()
