FROM python:3.12-slim
RUN apt-get update && apt-get install -y --no-install-recommends curl && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10002 worker
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY download_models.sh .
RUN sh download_models.sh /app/models
COPY tracker.py detector.py worker.py ./
ENV MODEL_PATH=/app/models/object_detection_yolox_2022nov.onnx \
    FACE_MODEL=/app/models/face_detection_yunet_2023mar.onnx \
    PYTHONUNBUFFERED=1
USER worker
CMD ["python", "worker.py"]
