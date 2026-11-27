#!/bin/sh
# Downloads the open-source models (Apache-2.0 / MIT).
#   sh download_models.sh [dir]           -> YOLOX-Nano (default, lowest RAM) + YuNet face detector
#   MODELS="nano tiny" sh download_models.sh   -> also YOLOX-Tiny (better at small objects such as phones)
set -eu
DIR="${1:-models}"
MODELS="${MODELS:-nano}"
mkdir -p "$DIR"
fetch() {
  if [ -s "$DIR/$2" ]; then echo "ok  $2"; return; fi
  echo "get $2"
  curl -fSL --retry 3 -o "$DIR/$2.part" "$1/$2" && mv "$DIR/$2.part" "$DIR/$2"
}
YOLOX=https://github.com/Megvii-BaseDetection/YOLOX/releases/download/0.1.1rc0
for m in $MODELS; do fetch "$YOLOX" "yolox_$m.onnx"; done
fetch https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_detection_yunet face_detection_yunet_2023mar.onnx
