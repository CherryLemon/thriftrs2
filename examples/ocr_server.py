#!/usr/bin/env python3
"""
OCR Thrift server example using thrift_rs_pyo3.

This example starts an OCRService server that handles:
  - detect_text(request: ImageRequest) -> OCRResult
  - detect_text_simple(image: bytes) -> OCRResult

Run the server:
    python examples/ocr_server.py

Then test it from another terminal by writing a small client or using thrift_rs_pyo3.load to call the service.
"""

import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'python'))

from thrift_rs_pyo3 import load, ThriftServer, TBufferedTransport

# ---------------------------------------------------------------------------
# Load the .thrift definition
# ---------------------------------------------------------------------------
THRIFT_FILE = os.path.join(os.path.dirname(__file__), "ocr_service.thrift")
thrift_module = load(THRIFT_FILE)
# Use getattr to avoid static analysis warnings about PyO3-added methods
service_def = getattr(thrift_module._parser, "get_service")("OCRService")
if service_def is None:
    raise SystemExit("Service 'OCRService' not found in the parsed thrift file")
# Convenience type aliases
BoundingBox = thrift_module.BoundingBox
TextSpan = thrift_module.TextSpan
OCRResult = thrift_module.OCRResult
ImageRequest = thrift_module.ImageRequest

# ---------------------------------------------------------------------------
# Handler implementations (stubs)
# Each handler receives keyword args matching the thrift method's parameters.
# Handlers may receive thrift struct objects with attributes accessible.
# Return a struct instance or a dict matching the return type.
# ---------------------------------------------------------------------------

def _get_image_from_request(request):
    # Support either object-like or dict-like request
    if request is None:
        return b""
    if hasattr(request, 'image'):
        return request.image
    try:
        return request['image']
    except Exception:
        return b""


def handle_detect_text(request) -> object:
    """Stub: pretend to detect a single text span in the image and return it.

    Args:
        request: ImageRequest struct-like object (fields: image, mime_type, ...)
    Returns:
        OCRResult struct instance.
    """
    print("  [ocr-server] detect_text called")
    start = time.time()

    image = _get_image_from_request(request)
    # In a real implementation you'd run OCR on `image` here.
    # We'll return a single fake detection. Interpret coords as normalized.

    bbox = BoundingBox(left=0.1, top=0.1, right=0.9, bottom=0.2, score=0.95)
    span = TextSpan(text="Detected text", confidence=0.95, language="en", bbox=bbox)

    elapsed_ms = int((time.time() - start) * 1000)
    result = OCRResult(spans=[span], processing_time_ms=elapsed_ms, engine="ocr-stub/0.1")
    print(f"  [ocr-server] returning 1 span, processing_time_ms={elapsed_ms}")
    return result


def handle_detect_text_simple(image: bytes) -> object:
    """Convenience handler that wraps raw bytes into an ImageRequest and calls the main handler."""
    req = ImageRequest(image=image)
    return handle_detect_text(req)

# ---------------------------------------------------------------------------
# Build and start the server
# ---------------------------------------------------------------------------

def main():
    server = ThriftServer(service_def, TBufferedTransport.transport_type)
    server.set_parser(thrift_module._parser)

    server.register_handler("detect_text", handle_detect_text)
    server.register_handler("detect_text_simple", handle_detect_text_simple)

    host, port = "127.0.0.1", 9091
    print(f"Starting OCRService on {host}:{port}  (Ctrl-C to stop)")
    server.serve(host, port)


if __name__ == "__main__":
    main()

