# GPU acceleration for the pdfwm watermark engine — TODO / runbook

**Status:** DEFERRED (chat server is being moved; do this on the new box).
**Goal:** run the TrustMark ONNX inference on an NVIDIA GPU instead of CPU.

## Why

The watermark embed/extract bottleneck is the TrustMark neural-net inference,
**not** rasterization or I/O. Measured on the current CPU (i7-12700K, 20 threads):

- Encoder forward pass = **~61 ms / tile** (256×256), fixed regardless of DPI.
- A page = 12 tiles → **~730 ms/page** of raw NN (the ~760 ms we observe).
- Already parallelized across tiles (rayon) and engine is cached across calls
  (`trustmark_engine`), so CPU is near its floor.

On an NVIDIA **RTX A2000 12 GB** (Ampere) a 256×256 conv inference is ~1–3 ms,
so expect **~10–30× on the NN** → embed ~tens of ms/page, and the leak-check
sliding-window decode (currently several seconds) drops to sub-second. GPU helps
**both** embed and extract.

## What's already true on the (old) box, for reference

- GPU present: `lspci` → `NVIDIA GA106 [RTX A2000 12GB]` (+ Intel UHD 770 iGPU).
- Driver installed (`nvidia-driver-580` / `nvidia-dkms-580` 580.159.03, DKMS-built
  for the running kernel) but **blocked by Secure Boot**: `modprobe nvidia` →
  *"Key was rejected by service"*.
- Root cause: the module is signed with the local MOK
  `/var/lib/shim-signed/mok/MOK.der` ("chat Secure Boot Module Signature key"),
  but that MOK is **NOT enrolled** in firmware (only Canonical's CA is).
- `nvidia-container-toolkit`: NOT installed. No `/usr/local/cuda`. Only ollama's
  bundled CUDA 11 runtime exists.
- A MOK import was staged then **revoked** during investigation — re-stage on the
  target box.

## Steps (in order; validate value before the heavy integration)

### 1. Load the driver (needs CONSOLE / IPMI access — Secure Boot)
There is **no SSH-only path**; enrollment happens in shim's MOK Manager at boot.
```sh
sudo mokutil --import /var/lib/shim-signed/mok/MOK.der   # set a one-time password
sudo reboot
# At the blue "MOK Management" screen: Enroll MOK → Continue → Yes → password → reboot
nvidia-smi    # must list the GPU
```
Alternative (also console-only): disable Secure Boot in BIOS, or
`sudo mokutil --disable-validation`. If the new box has Secure Boot OFF, the
DKMS module just loads — skip the enrollment.

### 2. PROVE the speedup before integrating (cheap, ~10 min)
```sh
# host-level python check — do NOT skip this gate
python3 -m pip install onnxruntime-gpu numpy   # (or uv pip)
# load models/encoder_Q.onnx with providers=['CUDAExecutionProvider'],
# run [1,3,256,256] + [1,100], time it vs CPU's ~61 ms.
```
Proceed only if it's ~10×+ faster. (Note: encoder ONNX is fixed batch=1, so it's
one inference per tile either way — see "future" below for dynamic-batch.)

### 3. Container GPU passthrough (podman)
```sh
sudo apt install nvidia-container-toolkit
sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml
# add to run-prod.sh / run-shadow.sh: --device nvidia.com/gpu=all
```
The image must also carry the CUDA 12 + cuDNN runtime libs the GPU onnxruntime
needs (bake into the Containerfile via prepare-context.sh, or mount from host).

### 4. Build pdfwm against the CUDA ONNX Runtime + patch TrustMark
- Switch the `ort` crate to a CUDA-enabled ONNX Runtime build/lib.
- **TrustMark hardcodes the CPU provider** — `Trustmark::new` builds its
  `Session`s with only optimization level + intra-threads, no execution provider.
  It must register `CUDAExecutionProvider` on the `SessionBuilder`. Options:
  fork trustmark 0.2.2 (small patch in `lib.rs::new`), or upstream a PR adding an
  execution-provider option. Then rebuild `libpdfwm.so` against the CUDA runtime.
- Keep CPU fallback: register CUDA EP first, CPU EP second, so it degrades
  gracefully if the GPU is unavailable.

### 5. Deploy + verify
- Blue-green as usual (`scripts/build.sh` → `run-shadow.sh` → `check-shadow.sh`
  → `switch-prod.sh`), but with the GPU device passed through.
- Verify: `podman exec staff-api-podman nvidia-smi` works, an embed round-trip
  decodes (votes=2), and embed time dropped to ~tens of ms/page.

## Future (optional, bigger)
- **Dynamic-batch ONNX**: the current `encoder_Q.onnx` is fixed batch=1, so all 12
  tiles run as separate inferences. Re-exporting TrustMark from PyTorch with a
  dynamic batch axis would let all tiles of a page run in **one** GPU call —
  another big win on top of GPU. Requires the upstream training/export pipeline.

## Caveats
- **Console/IPMI access is mandatory** for the Secure Boot step. Without it the
  GPU cannot be enabled (no SSH-only path).
- Match versions carefully: NVIDIA driver ↔ CUDA ↔ cuDNN ↔ onnxruntime-gpu ↔ ort.
- Unrelated, noticed during this work: `staff-api.service` was in
  `activating (auto-restart)` on the old box (live path via nginx→podman is fine);
  worth a look when setting up the new box.

## CPU baseline to beat (current, deployed)
- 1-page embed ≈ 0.7 s (Mac 12-core) / ~2 s cold on the old box.
- Engine caching (v0.1.6) removes the ~1 s per-export model reload after warmup.
- 600 DPI, tile_size 1536 (12 tiles), strength 0.65, JPEG 95.
