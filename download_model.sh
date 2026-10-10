#!/bin/bash
# Copyright 2026 Google LLC
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

set -e

# Change to script directory
cd "$(dirname "$0")"

# Activate virtual environment
if [ -f "venv/bin/activate" ]; then
    source venv/bin/activate
else
    echo "Virtual environment not found. Please run ./setup.sh first."
    exit 1
fi

MODEL_NAME="gemma4-e2b"

# Official LiteRT-LM build of Gemma 4 E2B-it published by Google AI Edge
# (Apache-2.0): https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm
# The plain .litertlm file is the generic CPU build (correct for Raspberry Pi);
# the *_Google_Tensor/*_intel/*_qualcomm variants are accelerator-specific.
HF_REPO="litert-community/gemma-4-E2B-it-litert-lm"
MODEL_FILE="gemma-4-E2B-it.litertlm"

echo "Checking if model '$MODEL_NAME' is already imported into litert-lm..."

# Exact match on the ID column (a plain substring grep would also match
# similarly-named models such as "gemma4-e2b-old").
if litert-lm list 2>/dev/null | awk -v m="$MODEL_NAME" '$1 == m { found = 1 } END { exit !found }'; then
    echo "Model '$MODEL_NAME' is already imported and ready to use!"
    exit 0
fi

# make models already downloads the same file into the model directory;
# import that copy instead of downloading it a second time.
LOCAL_MODEL="${MODEL_DIR:-models}/gemma/$MODEL_FILE"

# Importing copies ~2.6GB; downloading first needs another ~2.6GB for the
# Hugging Face cache. Require headroom before starting.
if [ -f "$LOCAL_MODEL" ]; then
    REQUIRED_GB=3
else
    REQUIRED_GB=6
fi
AVAIL_GB="$(df -Pk "$HOME" | awk 'NR==2 { print int($4 / 1024 / 1024) }')"
if [ -n "$AVAIL_GB" ] && [ "$AVAIL_GB" -lt "$REQUIRED_GB" ]; then
    echo "Error: only ${AVAIL_GB}GB free on the filesystem holding $HOME."
    echo "At least ${REQUIRED_GB}GB is needed to import the model."
    echo "Free up space and re-run this script."
    exit 1
fi

if [ -f "$LOCAL_MODEL" ]; then
    echo "Importing $LOCAL_MODEL as '$MODEL_NAME'..."
    litert-lm import "$LOCAL_MODEL" "$MODEL_NAME"
    echo "========================================="
    echo "Model import complete!"
    echo "You can now start the server with ./start.sh"
    echo "========================================="
    exit 0
fi

echo "Model '$MODEL_NAME' not found in local storage."
echo "Downloading and importing from Hugging Face repo: $HF_REPO..."
echo "This may take a few minutes depending on your internet connection."

litert-lm import --from-huggingface-repo "$HF_REPO" "$MODEL_FILE" "$MODEL_NAME"

echo "========================================="
echo "Model download and import complete!"
echo "You can now start the server with ./start.sh"
echo "========================================="
