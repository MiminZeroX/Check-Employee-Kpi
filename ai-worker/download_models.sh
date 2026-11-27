#!/bin/sh
# Downloads the default open-source models (both from OpenCV Zoo).
#   YOLOX object detector  - Apache-2.0
#   YuNet face detector    - MIT
set -eu
DIR="${1:-models}"
mkdir -p "$DIR"
BASE=https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models
fetch() {
  if [ -s "$DIR/$2" ]; then echo "ok  $2"; return; fi
  echo "get $2"
  curl -fSL --retry 3 -o "$DIR/$2.part" "$BASE/$1/$2" && mv "$DIR/$2.part" "$DIR/$2"
}
fetch object_detection_yolox object_detection_yolox_2022nov.onnx
fetch face_detection_yunet face_detection_yunet_2023mar.onnx
