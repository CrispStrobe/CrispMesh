#!/usr/bin/env python
"""Optional PyTorch/MPS sampling diagnostic; run each layout in a fresh process.

No model or third-party implementation is downloaded. This is a research backend
check, not part of the app or a reconstruction-quality test.
"""
import argparse
import json
import platform
from pathlib import Path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--layout', choices=('strided', 'contiguous'), required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    import torch
    import torch.nn.functional as functional

    if not torch.backends.mps.is_available():
        parser.error('this optional diagnostic requires PyTorch with an available MPS device')
    torch.set_num_threads(2)
    torch.manual_seed(1234)
    cases = []
    for batch, frames, size, channels in ((1, 4, 32, 128), (1, 4, 16, 256), (1, 78, 32, 128)):
        features = torch.randn(batch, frames, size, size, channels)
        queries = torch.rand(batch, 16, frames, 49, 2) * 2 - 1

        def views(feats, query):
            return (feats.permute(0, 1, 4, 2, 3).reshape(batch * frames, channels, size, size),
                    query.permute(0, 2, 1, 3, 4).reshape(batch * frames, 16, 49, 2))

        cpu_image, cpu_grid = views(features, queries)
        gpu_image, gpu_grid = views(features.to('mps'), queries.to('mps'))
        image_stride, grid_stride = list(gpu_image.stride()), list(gpu_grid.stride())
        if args.layout == 'contiguous':
            gpu_image, gpu_grid = gpu_image.contiguous(), gpu_grid.contiguous()
        with torch.inference_mode():
            expected = functional.grid_sample(cpu_image.contiguous(), cpu_grid.contiguous(),
                                              align_corners=False, padding_mode='zeros')
            actual = functional.grid_sample(gpu_image, gpu_grid,
                                            align_corners=False, padding_mode='zeros').cpu()
        error = (actual - expected).abs()
        cases.append({'shape_bthc': [batch, frames, size, channels],
                      'original_image_stride': image_stride, 'original_grid_stride': grid_stride,
                      'max_absolute_error': float(error.max()), 'mean_absolute_error': float(error.mean())})
    passed = all(case['max_absolute_error'] <= 2e-5 for case in cases)
    report = {'torch': torch.__version__, 'platform': platform.platform(), 'layout': args.layout,
              'fresh_process_required': True, 'tolerance': 2e-5, 'passed': passed, 'cases': cases,
              'scope': 'Sampling parity only; not full-model equivalence or reconstruction quality. '
                       'Mixed-layout runs can differ from isolated runs; compare fresh processes separately.'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))
    return 0 if passed else 1


if __name__ == '__main__':
    raise SystemExit(main())
