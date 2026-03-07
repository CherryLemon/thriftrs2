#!/usr/bin/env python3
"""
OCR Thrift client example for the OCRService defined in examples/ocr_service.thrift.

Usage:
    python examples/ocr_client.py [--image PATH] [--host HOST] [--port PORT]

If --image is omitted the client will send empty bytes to the server.
"""

import os
import argparse
import json
import traceback


from thriftrs2 import (
    load,
    make_client,
    TBufferedTransport,
    ThriftApplicationException,
)

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------
EXAMPLES_DIR = os.path.dirname(os.path.abspath(__file__))
THRIFT_FILE = os.path.join(EXAMPLES_DIR, "ocr_service.thrift")
DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 9091

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def parse_args():
    p = argparse.ArgumentParser(description="OCR client for OCRService")
    p.add_argument("--image", "-i", help="Path to image file to send (optional)")
    p.add_argument("--host", default=DEFAULT_HOST, help="Server host")
    p.add_argument("--port", default=DEFAULT_PORT, type=int, help="Server port")
    p.add_argument("--json", action="store_true", help="Output raw JSON-like representation")
    return p.parse_args()


def read_image_bytes(path):
    with open(path, "rb") as f:
        return f.read()


def _get_attr_or_key(obj, name, default=None):
    """Defensive helper to read attribute or dict key."""
    if obj is None:
        return default
    if hasattr(obj, name):
        return getattr(obj, name)
    try:
        return obj[name]
    except Exception:
        return default


def print_ocr_result(resp):
    if resp is None:
        print("No response from server")
        return

    spans = _get_attr_or_key(resp, 'spans', []) or []
    processing_time = _get_attr_or_key(resp, 'processing_time_ms', None)
    engine = _get_attr_or_key(resp, 'engine', None)

    print("OCR Result:")
    if processing_time is not None:
        print(f"  processing_time_ms: {processing_time}")
    if engine:
        print(f"  engine: {engine}")

    if not spans:
        print("  No spans detected")
        return

    for i, span in enumerate(spans, start=1):
        text = _get_attr_or_key(span, 'text', None)
        confidence = _get_attr_or_key(span, 'confidence', None)
        language = _get_attr_or_key(span, 'language', None)
        page = _get_attr_or_key(span, 'page', None)
        rotation = _get_attr_or_key(span, 'rotation', None)
        bbox = _get_attr_or_key(span, 'bbox', None)

        print(f"  Span {i}:")
        print(f"    text: {text}")
        if confidence is not None:
            print(f"    confidence: {confidence}")
        if language:
            print(f"    language: {language}")
        if page is not None:
            print(f"    page: {page}")
        if rotation is not None:
            print(f"    rotation: {rotation}")

        if bbox is not None:
            left = _get_attr_or_key(bbox, 'left', None)
            top = _get_attr_or_key(bbox, 'top', None)
            right = _get_attr_or_key(bbox, 'right', None)
            bottom = _get_attr_or_key(bbox, 'bottom', None)
            score = _get_attr_or_key(bbox, 'score', None)
            print("    bbox:")
            print(f"      left: {left}")
            print(f"      top: {top}")
            print(f"      right: {right}")
            print(f"      bottom: {bottom}")
            if score is not None:
                print(f"      score: {score}")


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    args = parse_args()

    try:
        thrift_module = load(THRIFT_FILE)
        # Obtain the ThriftService wrapper from the loaded module
        try:
            service = getattr(thrift_module, 'OCRService')
        except AttributeError:
            print("Could not find OCRService in the thrift file")
            return 2

        image_bytes = b''
        if args.image:
            try:
                image_bytes = read_image_bytes(args.image)
            except Exception as e:
                print(f"Failed to read image '{args.image}': {e}")
                return 2

        # Connect and call
        with make_client(
            service,
            args.host,
            args.port,
            TBufferedTransport.transport_type,
        ) as client:
            try:
                resp = client.call('detect_text_simple', image=image_bytes)
            except ThriftApplicationException as e:
                print(f"Server returned ThriftApplicationException: {e}")
                return 1
            except OSError as e:
                print(f"Network error / cannot reach server: {e}")
                return 1
            except Exception as e:
                print("Unexpected error while calling RPC:")
                traceback.print_exc()
                return 1

            if args.json:
                # Try to convert to a JSON-friendly dict
                try:
                    # Attempt naive serialization by walking fields
                    def to_serializable(obj):
                        if obj is None:
                            return None
                        if isinstance(obj, (str, int, float, bool)):
                            return obj
                        if isinstance(obj, (list, tuple)):
                            return [to_serializable(x) for x in obj]
                        # thrift struct-like: try attributes then mapping
                        result = {}
                        for k in dir(obj):
                            if k.startswith('_'):
                                continue
                            try:
                                v = getattr(obj, k)
                            except Exception:
                                continue
                            if callable(v):
                                continue
                            result[k] = to_serializable(v)
                        # fallback for mapping-like
                        if not result:
                            try:
                                for k, v in obj.items():
                                    result[k] = to_serializable(v)
                            except Exception:
                                pass
                        return result

                    serial = to_serializable(resp)
                    print(json.dumps(serial, indent=2))
                except Exception:
                    print("Failed to serialize response to JSON; falling back to pretty print")
                    print_ocr_result(resp)
            else:
                print_ocr_result(resp)

    except Exception:
        print("Failed to run client:")
        traceback.print_exc()
        return 2

    return 0


if __name__ == '__main__':
    raise SystemExit(main())

