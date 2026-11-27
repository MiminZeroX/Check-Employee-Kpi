FROM python:3.12-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10002 worker
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY download_models.sh .
ARG MODELS=nano
RUN MODELS="$MODELS" sh download_models.sh /app/models
COPY tracker.py detector.py stream.py worker.py ./
ENV MODEL_PATH=/app/models/yolox_nano.onnx \
    FACE_MODEL=/app/models/face_detection_yunet_2023mar.onnx \
    PYTHONUNBUFFERED=1 \
    LOW_MEMORY=1 \
    MALLOC_ARENA_MAX=2
USER worker
CMD ["python", "worker.py"]
