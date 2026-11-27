#!/bin/sh
# Downloads the open-source models (Apache-2.0 / MIT) and verifies their SHA-256,
# so a tampered or corrupted file is never used.
#   sh download_models.sh [dir]                 -> YOLOX-Nano (default, lowest RAM) + YuNet face detector
#   MODELS="nano tiny" sh download_models.sh    -> also YOLOX-Tiny (better at small objects such as phones)
set -eu
DIR="${1:-models}"
MODELS="${MODELS:-nano}"
mkdir -p "$DIR"

sha_of() {
  case "$1" in
    yolox_nano.onnx) echo c789161ed43c8269fcd4e67c67eeeb4e80c622da2eb296a20bc6007bd18a0b7d ;;
    yolox_tiny.onnx) echo 427cc366d34e27ff7a03e2899b5e3671425c262ea2291f88bb942bc1cc70b0f7 ;;
    face_detection_yunet_2023mar.onnx) echo 8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4 ;;
    *) echo "unknown model $1" >&2; exit 1 ;;
  esac
}

fetch() {
  want=$(sha_of "$2")
  if [ -s "$DIR/$2" ] && [ "$(sha256sum "$DIR/$2" | cut -d' ' -f1)" = "$want" ]; then echo "ok  $2"; return; fi
  echo "get $2"
  curl -fSL --proto '=https' --retry 3 -o "$DIR/$2.part" "$1/$2"
  got=$(sha256sum "$DIR/$2.part" | cut -d' ' -f1)
  if [ "$got" != "$want" ]; then
    rm -f "$DIR/$2.part"
    echo "SHA-256 mismatch for $2 (expected $want, got $got) - refusing to use it" >&2
    exit 1
  fi
  mv "$DIR/$2.part" "$DIR/$2"
}

YOLOX=https://github.com/Megvii-BaseDetection/YOLOX/releases/download/0.1.1rc0
for m in $MODELS; do fetch "$YOLOX" "yolox_$m.onnx"; done
fetch https://media.githubusercontent.com/media/opencv/opencv_zoo/main/models/face_detection_yunet face_detection_yunet_2023mar.onnx
