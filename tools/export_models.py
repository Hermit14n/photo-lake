#!/usr/bin/env python3
"""One-time model preparation for photolake's bird pipeline.

Produces, in --out (default ~/.local/share/photolake/models):
  detector.onnx            RT-DETR R50 (COCO, Apache-2.0); class 14 = bird
  bioclip2_visual.onnx     BioCLIP 2 image encoder (MIT), 224x224 -> 768-d embedding
  species.json             eBird species list (code, common, scientific, family, order)
  species_text.f32         L2-normalized text embeddings, float32 [n_species x dim], row order = species.json
  manifest.json            versions + shapes, read by the Rust loader

Setup (CPU-only torch is enough):
  python3 -m venv tools/.venv
  tools/.venv/bin/pip install --index-url https://download.pytorch.org/whl/cpu torch torchvision
  tools/.venv/bin/pip install open_clip_torch onnx onnxruntime huggingface_hub requests numpy
  tools/.venv/bin/python tools/export_models.py
"""

import argparse
import csv
import io
import json
import os
import shutil
import time
from pathlib import Path

import numpy as np
import requests

DETECTOR_REPO = "onnx-community/rtdetr_r50vd"
BIOCLIP = "hf-hub:imageomics/bioclip-2"
TAXONOMY_URL = "https://api.ebird.org/v2/ref/taxonomy/ebird?fmt=csv&cat=species"
TEMPLATES = [
    "a photo of {common}.",
    "a photo of {sci}.",
    "a photo of {common}, a type of bird, {sci}.",
]


def default_out() -> Path:
    base = os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")
    return Path(base) / "photolake" / "models"


def fetch_detector(out: Path) -> None:
    from huggingface_hub import hf_hub_download

    dst = out / "detector.onnx"
    if dst.exists():
        print(f"detector: {dst} exists")
        return
    src = hf_hub_download(DETECTOR_REPO, "onnx/model.onnx")
    shutil.copyfile(src, dst)
    print(f"detector: saved {dst} ({dst.stat().st_size / 1e6:.0f} MB)")


def fetch_taxonomy() -> list[dict]:
    resp = requests.get(TAXONOMY_URL, timeout=120)
    resp.raise_for_status()
    rows = []
    for r in csv.DictReader(io.StringIO(resp.text)):
        if r.get("EXTINCT", "").strip().lower() in ("true", "1", "yes"):
            continue
        rows.append(
            {
                "code": r["SPECIES_CODE"],
                "common": r["COMMON_NAME"],
                "sci": r["SCIENTIFIC_NAME"],
                "family": r["FAMILY_COM_NAME"],
                "order": r["ORDER"],
            }
        )
    return rows


def export_bioclip(out: Path, species: list[dict], batch: int) -> dict:
    import open_clip
    import torch

    torch.set_grad_enabled(False)
    # The fused attention kernel has no ONNX symbolic; use the plain implementation.
    torch.backends.mha.set_fastpath_enabled(False)
    model, _, preprocess = open_clip.create_model_and_transforms(BIOCLIP)
    tokenizer = open_clip.get_tokenizer(BIOCLIP)
    model.eval()

    # Preprocessing constants the Rust side must replicate.
    norm = next(t for t in preprocess.transforms if t.__class__.__name__ == "Normalize")
    size = model.visual.image_size
    size = size[0] if isinstance(size, (tuple, list)) else size

    visual_path = out / "bioclip2_visual.onnx"
    if not visual_path.exists():

        class Visual(torch.nn.Module):
            def __init__(self, m):
                super().__init__()
                self.m = m

            def forward(self, pixels):
                f = self.m.encode_image(pixels)
                return f / f.norm(dim=-1, keepdim=True)

        dummy = torch.randn(1, 3, size, size)
        torch.onnx.export(
            Visual(model),
            (dummy,),
            str(visual_path),
            input_names=["pixels"],
            output_names=["embedding"],
            dynamic_axes={"pixels": {0: "n"}, "embedding": {0: "n"}},
            opset_version=17,
            dynamo=False,
        )
        print(f"bioclip: saved {visual_path} ({visual_path.stat().st_size / 1e6:.0f} MB)")

    emb_path = out / "species_text.f32"
    dim = None
    if not emb_path.exists():
        t0 = time.time()
        chunks = []
        for i in range(0, len(species), batch):
            part = species[i : i + batch]
            acc = None
            for tpl in TEMPLATES:
                toks = tokenizer([tpl.format(**s) for s in part])
                f = model.encode_text(toks)
                f = f / f.norm(dim=-1, keepdim=True)
                acc = f if acc is None else acc + f
            acc = acc / acc.norm(dim=-1, keepdim=True)
            chunks.append(acc.numpy().astype(np.float32))
            done = min(i + batch, len(species))
            print(f"\rtext embeddings: {done}/{len(species)} ({time.time() - t0:.0f}s)", end="", flush=True)
        print()
        mat = np.concatenate(chunks)
        mat.tofile(emb_path)
        dim = mat.shape[1]
    else:
        dim = emb_path.stat().st_size // 4 // len(species)

    return {
        "image_size": int(size),
        "mean": [float(x) for x in norm.mean],
        "std": [float(x) for x in norm.std],
        "dim": int(dim),
        "logit_scale": float(model.logit_scale.exp().item()),
    }


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", type=Path, default=default_out())
    ap.add_argument("--batch", type=int, default=256)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)

    fetch_detector(args.out)

    species_path = args.out / "species.json"
    if species_path.exists():
        species = json.loads(species_path.read_text())
    else:
        species = fetch_taxonomy()
        species_path.write_text(json.dumps(species))
    print(f"taxonomy: {len(species)} species")

    clip = export_bioclip(args.out, species, args.batch)

    manifest = {
        "version": "rtdetr_r50vd+bioclip2/1",
        "detector": {
            "file": "detector.onnx",
            "input_size": 640,
            "bird_class": 14,
            "source": DETECTOR_REPO,
        },
        "classifier": {"file": "bioclip2_visual.onnx", "source": BIOCLIP, **clip},
        "species": {"file": "species.json", "embeddings": "species_text.f32", "count": len(species)},
        "templates": TEMPLATES,
    }
    (args.out / "manifest.json").write_text(json.dumps(manifest, indent=2))
    print(f"done: {args.out}")


if __name__ == "__main__":
    main()
