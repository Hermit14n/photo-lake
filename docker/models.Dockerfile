# syntax=docker/dockerfile:1.7
# Bird model bundle (RT-DETR detector + BioCLIP 2 + eBird species embeddings), ~1.4 GB.
# Built once per model version; the `models` compose service copies it into the data folder.
FROM python:3.12-slim AS export
RUN pip install --no-cache-dir --index-url https://download.pytorch.org/whl/cpu torch torchvision \
 && pip install --no-cache-dir open_clip_torch onnx onnxruntime huggingface_hub requests numpy
COPY tools/export_models.py /export_models.py
RUN python /export_models.py --out /models

FROM busybox:1.37
COPY --from=export /models /models
COPY docker/install-models.sh /install-models.sh
ENTRYPOINT ["/bin/sh", "/install-models.sh"]
