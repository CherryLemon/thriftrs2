// OCR service Thrift definition
// Provides a simple OCR API that accepts an image (bytes) and returns
// detected text spans with bounding boxes, confidence, language, etc.

namespace py ocr_service

/** Coordinate mode for bounding boxes. PIXEL = absolute pixel coords,
 *  NORMALIZED = [0.0 .. 1.0] relative coords. */
enum CoordMode {
  PIXEL = 0,
  NORMALIZED = 1
}

/** Bounding box coordinates. Interpretation depends on ImageRequest.coord_mode. */
struct BoundingBox {
  1: double left    // X minimum (left)
  2: double top     // Y minimum (top)
  3: double right   // X maximum (right)
  4: double bottom  // Y maximum (bottom)
  5: optional double score // optional bbox confidence / score
}

/** A single detected text span (word/line/segment). */
struct TextSpan {
  1: string text
  2: double confidence
  3: optional string language
  4: BoundingBox bbox
  5: optional i32 page    // page number for multi-page documents (0-based)
  6: optional double rotation // rotation in degrees (clockwise)
}

/** Top-level OCR result. */
struct OCRResult {
  1: list<TextSpan> spans
  2: optional i64 processing_time_ms
  3: optional string engine  // optional engine/version identifier
}

/** Image request wrapper. */
struct ImageRequest {
  1: binary image
  2: optional string mime_type      // e.g. "image/png", "image/jpeg"
  3: optional CoordMode coord_mode = CoordMode.NORMALIZED
  4: optional double min_confidence = 0.0
  5: optional bool detect_orientation = true
  6: optional i32 page_number = 0    // for multi-page formats like PDF
}

service OCRService {
  /** Full request with options. */
  OCRResult detect_text(1: ImageRequest request)

  /** Convenience method accepting raw image bytes (defaults used). */
  OCRResult detect_text_simple(1: binary image)
}

