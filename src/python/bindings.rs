use pyo3::prelude::*;
use pyo3::types::{PyDict, PyBytes, PyList};
use pyo3::Py;
use crate::parser::{Parser, ast::*};
use crate::protocol::{BinaryProtocolReader, BinaryProtocolWriter, TType, FieldBegin,
                      MESSAGE_TYPE_REPLY, MESSAGE_TYPE_EXCEPTION};
use std::collections::HashMap;
use std::io::Cursor;
use byteorder::BigEndian;
// ──────────────────────────────────────────────────────────────────────────────
// ThriftParser
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct ThriftParser {
    document: Option<ThriftDocument>,
}

#[pymethods]
impl ThriftParser {
    #[new]
    pub fn new() -> Self {
        Self { document: None }
    }

    pub fn parse(&mut self, content: &str) -> PyResult<()> {
        let mut parser = Parser::new(content)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e)))?;

        self.document = Some(parser.parse_document()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Parse error: {}", e)))?);

        Ok(())
    }

    pub fn list_structs(&self) -> PyResult<Vec<String>> {
        match &self.document {
            Some(doc) => Ok(doc.structs.keys().cloned().collect()),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }

    pub fn get_struct(&self, name: &str) -> PyResult<Option<ThriftStruct>> {
        match &self.document {
            Some(doc) => {
                Ok(doc.structs.get(name).map(|s| {
                    let fields: Vec<ThriftField> = s.fields.iter().map(|f| ThriftField {
                        id: f.id,
                        name: f.name.clone(),
                        required: f.required,
                        field_type: f.field_type.clone(),
                    }).collect();
                    let field_map: HashMap<i16, usize> = fields.iter()
                        .enumerate()
                        .map(|(idx, f)| (f.id, idx))
                        .collect();
                    ThriftStruct {
                        name: s.name.clone(),
                        fields,
                        field_map,
                    }
                }))
            }
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }

    pub fn list_services(&self) -> PyResult<Vec<String>> {
        match &self.document {
            Some(doc) => Ok(doc.services.keys().cloned().collect()),
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }

    pub fn get_service(&self, name: &str) -> PyResult<Option<PyThriftService>> {
        match &self.document {
            Some(doc) => {
                Ok(doc.services.get(name).map(|svc| {
                    let methods: Vec<PyThriftMethod> = svc.methods.iter().map(|m| {
                        let args: Vec<ThriftField> = m.arguments.iter().map(|f| ThriftField {
                            id: f.id,
                            name: f.name.clone(),
                            required: f.required,
                            field_type: f.field_type.clone(),
                        }).collect();
                        let exceptions: Vec<ThriftField> = m.exceptions.iter().map(|f| ThriftField {
                            id: f.id,
                            name: f.name.clone(),
                            required: f.required,
                            field_type: f.field_type.clone(),
                        }).collect();
                        PyThriftMethod {
                            name: m.name.clone(),
                            return_type: m.return_type.clone(),
                            arguments: args,
                            exceptions,
                        }
                    }).collect();
                    PyThriftService {
                        name: svc.name.clone(),
                        methods,
                    }
                }))
            }
            None => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>("No document parsed yet")),
        }
    }

    /// Return a snapshot of the whole parsed document's struct map so the server
    /// can resolve nested struct types by name during serialisation/deserialisation.
    pub(crate) fn struct_map(&self) -> HashMap<String, ThriftStruct> {
        match &self.document {
            Some(doc) => doc.structs.iter().map(|(k, s)| {
                let fields: Vec<ThriftField> = s.fields.iter().map(|f| ThriftField {
                    id: f.id,
                    name: f.name.clone(),
                    required: f.required,
                    field_type: f.field_type.clone(),
                }).collect();
                let field_map: HashMap<i16, usize> = fields.iter()
                    .enumerate()
                    .map(|(idx, f)| (f.id, idx))
                    .collect();
                (k.clone(), ThriftStruct { name: s.name.clone(), fields, field_map })
            }).collect(),
            None => HashMap::new(),
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftStruct / ThriftField
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass(skip_from_py_object)]
#[derive(Clone)]
pub struct ThriftStruct {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub fields: Vec<ThriftField>,
    /// Cached map from field id -> index in `fields`, built once at parse time.
    field_map: HashMap<i16, usize>,
}

#[pymethods]
impl ThriftStruct {
    pub fn serialize(&self, data: &Bound<'_, PyDict>) -> PyResult<Vec<u8>> {
        let mut buffer = Vec::with_capacity(128 + self.fields.len() * 16);
        let mut writer = BinaryProtocolWriter::new(&mut buffer);
        serialize_struct_fields(&mut writer, &self.fields, data, &HashMap::new())?;
        writer.write_field_stop()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
        Ok(buffer)
    }

    pub fn deserialize<'py>(&self, py: Python<'py>, data: &[u8]) -> PyResult<Bound<'py, PyDict>> {
        let mut cursor = Cursor::new(data);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        deserialize_struct_fields(&mut reader, &self.fields, &self.field_map, &HashMap::new(), py)
    }

    pub fn __repr__(&self) -> String {
        format!("ThriftStruct(name={:?}, fields={:?})", self.name, self.fields)
    }
}

#[pyclass(skip_from_py_object)]
#[derive(Debug, Clone)]
pub struct ThriftField {
    #[pyo3(get)]
    pub id: i16,
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub required: bool,
    pub(crate) field_type: ThriftType,
}

#[pymethods]
impl ThriftField {
    pub fn __repr__(&self) -> String {
        format!("ThriftField(id={}, name={:?}, required={}, field_type={:?})", self.id, self.name, self.required, self.field_type)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// PyThriftService / PyThriftMethod  (exposed to Python)
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
#[derive(Clone)]
pub struct PyThriftService {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub methods: Vec<PyThriftMethod>,
}

#[pymethods]
impl PyThriftService {
    pub fn get_method(&self, name: &str) -> Option<PyThriftMethod> {
        self.methods.iter().find(|m| m.name == name).cloned()
    }

    pub fn __repr__(&self) -> String {
        let method_names: Vec<&str> = self.methods.iter().map(|m| m.name.as_str()).collect();
        format!("ThriftService(name={:?}, methods={:?})", self.name, method_names)
    }
}

#[pyclass]
#[derive(Clone)]
pub struct PyThriftMethod {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub arguments: Vec<ThriftField>,
    #[pyo3(get)]
    pub exceptions: Vec<ThriftField>,
    pub(crate) return_type: ThriftType,
}

#[pymethods]
impl PyThriftMethod {
    pub fn __repr__(&self) -> String {
        format!("ThriftMethod(name={:?}, return_type={:?})", self.name, self.return_type)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// BinaryProtocol
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct BinaryProtocol;

#[pymethods]
impl BinaryProtocol {
    #[new]
    pub fn new() -> Self {
        Self
    }

    #[staticmethod]
    pub fn serialize_struct(struct_def: &ThriftStruct, data: &Bound<'_, PyDict>) -> PyResult<Vec<u8>> {
        struct_def.serialize(data)
    }

    #[staticmethod]
    pub fn deserialize_struct<'py>(py: Python<'py>, struct_def: &ThriftStruct, data: &[u8]) -> PyResult<Bound<'py, PyAny>> {
        struct_def.deserialize(py, data).map(|d| d.into_any())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// TransportType
// ──────────────────────────────────────────────────────────────────────────────

/// Transport layer to use when reading/writing Thrift messages over TCP.
///
/// * `Framed`   – each message is prefixed by a 4-byte big-endian frame length
///                (TFramedTransport in the official Thrift SDKs).
/// * `Buffered` – messages are written directly to the socket with no framing
///                envelope (TBufferedTransport / TSocket in the official SDKs).
#[pyclass(eq, eq_int)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportType {
    Framed = 0,
    Buffered = 1,
}

#[pymethods]
impl TransportType {
    #[new]
    pub fn new() -> Self {
        Self::Framed
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftServer
// ──────────────────────────────────────────────────────────────────────────────

#[pyclass]
pub struct ThriftServer {
    service: PyThriftService,
    handlers: HashMap<String, Py<PyAny>>,
    /// Snapshot of all structs from the parsed document, needed to resolve
    /// named struct types during handler argument de/serialisation.
    struct_map: HashMap<String, ThriftStruct>,
    /// Transport type to use (framed or buffered).
    transport: TransportType,
}

#[pymethods]
impl ThriftServer {
    #[new]
    #[pyo3(signature = (service, transport = TransportType::Framed))]
    pub fn new(service: PyThriftService, transport: TransportType) -> Self {
        Self {
            service,
            handlers: HashMap::new(),
            struct_map: HashMap::new(),
            transport,
        }
    }

    /// Get the current transport type.
    #[getter]
    pub fn transport(&self) -> TransportType {
        self.transport
    }

    /// Set the transport type (framed or buffered).
    #[setter]
    pub fn set_transport(&mut self, transport: TransportType) {
        self.transport = transport;
    }

    /// Attach the parser so that nested struct types can be resolved.
    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    /// Register a Python callable as the handler for `method_name`.
    pub fn register_handler(&mut self, method_name: &str, handler: Py<PyAny>) {
        self.handlers.insert(method_name.to_string(), handler);
    }

    /// Start a **blocking** TCP server on `host:port`.
    /// Each connection is handled in its own OS thread.
    /// Releases the GIL while waiting for connections.
    pub fn serve(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;
        use std::sync::Arc;

        let addr = format!("{}:{}", host, port);

        // Clone handlers while we still hold the GIL.
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self.handlers.iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = self.struct_map.clone();
        let transport = self.transport;

        let listener = TcpListener::bind(&addr)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e)))?;

        println!("ThriftServer ({:?}) listening on {}", transport, addr);

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);
        let struct_map = Arc::new(struct_map);

        // Release the GIL while blocking in accept loop.
        py.detach(|| {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let service = Arc::clone(&service);
                        let handlers = Arc::clone(&handlers);
                        let struct_map = Arc::clone(&struct_map);
                        std::thread::spawn(move || {
                            if let Err(e) = handle_connection(stream, &service, &handlers, &struct_map, transport) {
                                eprintln!("Connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => eprintln!("Accept error: {}", e),
                }
            }
        });

        Ok(())
    }

    /// Start a **non-blocking** server in a background thread.
    /// Returns immediately. The server keeps running until the process exits.
    pub fn serve_nonblocking(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;
        use std::sync::Arc;

        let addr = format!("{}:{}", host, port);
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self.handlers.iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = self.struct_map.clone();
        let transport = self.transport;

        let listener = TcpListener::bind(&addr)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e)))?;

        println!("ThriftServer (non-blocking, {:?}) listening on {}", transport, addr);

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);
        let struct_map = Arc::new(struct_map);

        std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let service = Arc::clone(&service);
                        let handlers = Arc::clone(&handlers);
                        let struct_map = Arc::clone(&struct_map);
                        std::thread::spawn(move || {
                            if let Err(e) = handle_connection(stream, &service, &handlers, &struct_map, transport) {
                                eprintln!("Connection error: {}", e);
                            }
                        });
                    }
                    Err(e) => eprintln!("Accept error: {}", e),
                }
            }
        });

        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Connection handler
// ──────────────────────────────────────────────────────────────────────────────

fn handle_connection(
    mut stream: std::net::TcpStream,
    service: &PyThriftService,
    handlers: &HashMap<String, Py<PyAny>>,
    struct_map: &HashMap<String, ThriftStruct>,
    transport: TransportType,
) -> std::io::Result<()> {
    use std::io::{Read, BufReader};
    use byteorder::{ReadBytesExt, WriteBytesExt};

    // For buffered transport we wrap the stream in a BufReader so we can do
    // efficient byte-at-a-time reads without syscall overhead, while still
    // sharing the same underlying TCP stream for writes.
    //
    // We use an enum to avoid monomorphisation or Box<dyn Read> overhead in
    // the common case; both arms dispatch the same logic below.
    enum StreamReader {
        Framed(std::net::TcpStream),
        Buffered(BufReader<std::net::TcpStream>),
    }

    // We need the raw stream for writes; keep a separate clone/reference.
    // TcpStream can be cloned to give independent read/write ends.
    let write_stream = stream.try_clone()
        .map_err(|e| std::io::Error::new(e.kind(), format!("clone stream: {}", e)))?;
    let mut write_stream = write_stream;

    let mut reader_holder = match transport {
        TransportType::Framed => StreamReader::Framed(stream),
        TransportType::Buffered => {
            // Wrap the original stream in a BufReader for buffered reads.
            StreamReader::Buffered(BufReader::new(stream))
        }
    };

    loop {
        // ── Read the next message payload ────────────────────────────────────
        let frame: Vec<u8> = match &mut reader_holder {
            StreamReader::Framed(s) => {
                // Framed transport: 4-byte big-endian length prefix followed by payload.
                let frame_len = match s.read_i32::<BigEndian>() {
                    Ok(n) if n > 0 => n as usize,
                    Ok(_) => return Ok(()),   // graceful close / zero-length
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                };
                let mut buf = vec![0u8; frame_len];
                s.read_exact(&mut buf)?;
                buf
            }
            StreamReader::Buffered(r) => {
                // Buffered transport: no framing envelope.  We peek at the
                // first 4 bytes to detect the Thrift binary protocol magic
                // (0x80 0x01 ...) and then read the full message by re-parsing
                // the header to find the total on-wire length.
                //
                // Strategy: read the 4-byte version/type word first, then the
                // method-name length, then the name, then seq_id (4 bytes),
                // then the struct body up to the final STOP byte.  We
                // reassemble everything into a contiguous buffer so the rest
                // of the dispatch path is identical to the framed branch.

                // 1. Read version+type (4 bytes)
                let mut hdr = [0u8; 4];
                match r.read_exact(&mut hdr) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                }

                // 2. Read name length (4 bytes) + name + seq_id (4 bytes)
                let mut name_len_bytes = [0u8; 4];
                r.read_exact(&mut name_len_bytes)?;
                let name_len = i32::from_be_bytes(name_len_bytes) as usize;
                let mut name_buf = vec![0u8; name_len];
                r.read_exact(&mut name_buf)?;
                let mut seq_id_bytes = [0u8; 4];
                r.read_exact(&mut seq_id_bytes)?;

                // 3. Slurp the struct body until we hit STOP (0x00) at depth 0.
                //    We track nesting depth to handle nested structs.
                let mut body: Vec<u8> = Vec::with_capacity(256);
                read_buffered_struct_body(r, &mut body)?;

                // 4. Reassemble the full frame.
                let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
                frame.extend_from_slice(&hdr);
                frame.extend_from_slice(&name_len_bytes);
                frame.extend_from_slice(&name_buf);
                frame.extend_from_slice(&seq_id_bytes);
                frame.extend_from_slice(&body);
                frame
            }
        };

        // ── Parse message header ─────────────────────────────────────────────
        let mut cursor = Cursor::new(&frame[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        let msg_begin = reader.read_message_begin()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        // ── Find method definition ────────────────────────────────────────────
        let method_def = service.methods.iter().find(|m| m.name == msg_begin.name);

        let response_payload = match method_def {
            None => {
                build_exception_reply(&msg_begin.name, msg_begin.seq_id,
                    1, &format!("Unknown method: {}", msg_begin.name))
            }
            Some(method) => {
                // ── Deserialise arguments ─────────────────────────────────────
                let arg_field_map: HashMap<i16, usize> = method.arguments.iter()
                    .enumerate()
                    .map(|(i, f)| (f.id, i))
                    .collect();

                let args_dict: Py<PyDict> = Python::attach(|py| {
                    deserialize_struct_fields(&mut reader, &method.arguments, &arg_field_map, struct_map, py)
                        .map(|d| d.unbind())
                }).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

                // ── Call Python handler ───────────────────────────────────────
                let handler = match handlers.get(&msg_begin.name) {
                    Some(h) => h,
                    None => {
                        let payload = build_exception_reply(&msg_begin.name, msg_begin.seq_id,
                            1, &format!("No handler registered for: {}", msg_begin.name));
                        send_response(&mut write_stream, &payload, transport)?;
                        continue;
                    }
                };

                let result: Result<Vec<u8>, String> = Python::attach(|py| {
                    let py_args_dict = args_dict.bind(py);
                    let result = handler.call(py, (), Option::from(py_args_dict))?;
                    let reply_body = build_reply_body(py, &method.return_type, result.bind(py), struct_map)?;
                    Ok(build_reply_frame(&msg_begin.name, msg_begin.seq_id, &reply_body))
                }).map_err(|e: PyErr| e.to_string());

                match result {
                    Ok(frame) => frame,
                    Err(err_msg) => build_exception_reply(&msg_begin.name, msg_begin.seq_id, 6, &err_msg),
                }
            }
        };

        send_response(&mut write_stream, &response_payload, transport)?;
    }
}

fn send_response(stream: &mut std::net::TcpStream, payload: &[u8], transport: TransportType) -> std::io::Result<()> {
    use byteorder::{BigEndian, WriteBytesExt};
    use std::io::Write;
    match transport {
        TransportType::Framed => {
            stream.write_i32::<BigEndian>(payload.len() as i32)?;
            stream.write_all(payload)?;
        }
        TransportType::Buffered => {
            stream.write_all(payload)?;
        }
    }
    stream.flush()
}

/// Read a Thrift struct body (including nested structs) from a buffered reader
/// until the outermost STOP byte (0x00) is consumed, appending all bytes
/// (including the final STOP) to `out`.
///
/// This is used by the buffered-transport path to reassemble a complete
/// on-wire message before handing it to the normal cursor-based parser.
fn read_buffered_struct_body<R: std::io::Read>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;

    loop {
        // Read the field type byte.
        let field_type_byte = reader.read_u8()?;
        out.push(field_type_byte);

        if field_type_byte == 0x00 {
            // STOP field — end of this struct.
            return Ok(());
        }

        // Read the 2-byte field id.
        let id_hi = reader.read_u8()?;
        let id_lo = reader.read_u8()?;
        out.push(id_hi);
        out.push(id_lo);

        // Read the field value according to its type.
        read_buffered_value(reader, field_type_byte, out)?;
    }
}

/// Read a single Thrift value of the given wire type into `out`.
fn read_buffered_value<R: std::io::Read>(
    reader: &mut R,
    field_type_byte: u8,
    out: &mut Vec<u8>,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;

    match field_type_byte {
        // BOOL, BYTE
        0x02 | 0x03 => {
            let b = reader.read_u8()?;
            out.push(b);
        }
        // I16
        0x06 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // I32
        0x08 => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // I64, DOUBLE
        0x0a | 0x04 => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        // STRING / BINARY  (4-byte length prefix + data)
        0x0b => {
            let mut len_bytes = [0u8; 4];
            reader.read_exact(&mut len_bytes)?;
            out.extend_from_slice(&len_bytes);
            let len = u32::from_be_bytes(len_bytes) as usize;
            let start = out.len();
            out.resize(start + len, 0);
            reader.read_exact(&mut out[start..])?;
        }
        // STRUCT (recursive)
        0x0c => {
            read_buffered_struct_body(reader, out)?;
        }
        // MAP
        0x0d => {
            let mut header = [0u8; 6]; // key_type(1) + val_type(1) + size(4)
            reader.read_exact(&mut header)?;
            out.extend_from_slice(&header);
            let key_type = header[0];
            let val_type = header[1];
            let size = i32::from_be_bytes([header[2], header[3], header[4], header[5]]);
            for _ in 0..size {
                read_buffered_value(reader, key_type, out)?;
                read_buffered_value(reader, val_type, out)?;
            }
        }
        // LIST, SET
        0x0f | 0x0e => {
            let mut header = [0u8; 5]; // elem_type(1) + size(4)
            reader.read_exact(&mut header)?;
            out.extend_from_slice(&header);
            let elem_type = header[0];
            let size = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            for _ in 0..size {
                read_buffered_value(reader, elem_type, out)?;
            }
        }
        _ => {
            // Unknown type; we can't safely advance — return an error.
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unknown field type byte 0x{:02x} in buffered stream", field_type_byte),
            ));
        }
    }
    Ok(())
}

/// Serialise the return value of a method call into a reply struct body.
/// The result field has id=0; success goes in field 0.
fn build_reply_body(
    py: Python<'_>,
    return_type: &ThriftType,
    value: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    let mut writer = BinaryProtocolWriter::new(&mut buf);

    match return_type {
        ThriftType::Struct(name) if name == "void" => {
            // void — just write STOP
        }
        _ => {
            // write field 0 = success result
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(return_type),
                id: 0,
            };
            writer.write_field_begin(&field_begin)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            write_value_with_structs(&mut writer, return_type, value, struct_map)?;
        }
    }

    writer.write_field_stop()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    drop(writer);
    let _ = py; // py is used implicitly by pyo3 error types above
    Ok(buf)
}

/// Wrap a reply body into a full Thrift Binary Protocol message frame (no outer i32 length).
fn build_reply_frame(method_name: &str, seq_id: i32, body: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(32 + body.len());
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer.write_message_begin(method_name, MESSAGE_TYPE_REPLY, seq_id).unwrap();
    }
    buf.extend_from_slice(body);
    buf
}

/// Build an application-exception reply frame.
fn build_exception_reply(method_name: &str, seq_id: i32, ex_type: i32, message: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer.write_message_begin(method_name, MESSAGE_TYPE_EXCEPTION, seq_id).unwrap();
        // TApplicationException struct: field 1 = message (string), field 2 = type (i32)
        let msg_field = FieldBegin { name: None, field_type: TType::String, id: 1 };
        writer.write_field_begin(&msg_field).unwrap();
        writer.write_string(message).unwrap();
        let type_field = FieldBegin { name: None, field_type: TType::I32, id: 2 };
        writer.write_field_begin(&type_field).unwrap();
        writer.write_i32(ex_type).unwrap();
        writer.write_field_stop().unwrap();
    }
    buf
}

// ──────────────────────────────────────────────────────────────────────────────
// Helpers shared by ThriftStruct and server handler
// ──────────────────────────────────────────────────────────────────────────────

pub(crate) fn serialize_struct_fields<W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    fields: &[ThriftField],
    data: &Bound<'_, PyDict>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    for field in fields {
        if let Some(value) = data.get_item(&field.name)? {
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(&field.field_type),
                id: field.id,
            };
            writer.write_field_begin(&field_begin)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Write error: {}", e)))?;
            write_value_with_structs(writer, &field.field_type, &value, struct_map)?;
        }
    }
    Ok(())
}

pub(crate) fn deserialize_struct_fields<'py, R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    fields: &[ThriftField],
    field_map: &HashMap<i16, usize>,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Bound<'py, PyDict>> {
    let result = PyDict::new(py);
    loop {
        let field_begin = reader.read_field_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Read error: {}", e)))?;
        if field_begin.field_type == TType::Stop {
            break;
        }
        if let Some(&idx) = field_map.get(&field_begin.id) {
            let field = &fields[idx];
            let value = read_value_with_structs(reader, &field.field_type, struct_map, py)?;
            result.set_item(&field.name, value)?;
        } else {
            skip_value(reader, field_begin.field_type)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(format!("Skip error: {}", e)))?;
        }
    }
    Ok(result)
}

// ──────────────────────────────────────────────────────────────────────────────
// Type helpers
// ──────────────────────────────────────────────────────────────────────────────

#[inline]
pub(crate) fn thrift_type_to_ttype(thrift_type: &ThriftType) -> TType {
    match thrift_type {
        ThriftType::Bool => TType::Bool,
        ThriftType::Byte => TType::Byte,
        ThriftType::I16 => TType::I16,
        ThriftType::I32 => TType::I32,
        ThriftType::I64 => TType::I64,
        ThriftType::Double => TType::Double,
        ThriftType::String => TType::String,
        ThriftType::Binary => TType::String,
        ThriftType::List(_) => TType::List,
        ThriftType::Set(_) => TType::Set,
        ThriftType::Map(_, _) => TType::Map,
        ThriftType::Struct(_) => TType::Struct,
    }
}

fn write_value_with_structs<'py, W: std::io::Write>(
    writer: &mut BinaryProtocolWriter<W>,
    thrift_type: &ThriftType,
    value: &Bound<'py, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    match thrift_type {
        ThriftType::Bool => {
            writer.write_bool(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Byte => {
            writer.write_byte(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I16 => {
            writer.write_i16(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I32 => {
            writer.write_i32(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::I64 => {
            writer.write_i64(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Double => {
            writer.write_double(value.extract()?)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::String => {
            let val: String = value.extract()?;
            writer.write_string(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::Binary => {
            let val: Vec<u8> = value.extract()?;
            writer.write_binary(&val)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
        ThriftType::List(elem_type) | ThriftType::Set(elem_type) => {
            use crate::protocol::ListBegin;
            let list = value.cast::<PyList>()?;
            let lb = ListBegin {
                element_type: thrift_type_to_ttype(elem_type),
                size: list.len() as i32,
            };
            if matches!(thrift_type, ThriftType::List(_)) {
                writer.write_list_begin(&lb)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            } else {
                use crate::protocol::SetBegin;
                writer.write_set_begin(&SetBegin { element_type: lb.element_type, size: lb.size })
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }
            for item in list.iter() {
                write_value_with_structs(writer, elem_type, &item, struct_map)?;
            }
        }
        ThriftType::Map(key_type, val_type) => {
            use pyo3::types::PyDict as PyDictType;
            use crate::protocol::MapBegin;
            let dict = value.cast::<PyDictType>()?;
            let mb = MapBegin {
                key_type: thrift_type_to_ttype(key_type),
                value_type: thrift_type_to_ttype(val_type),
                size: dict.len() as i32,
            };
            writer.write_map_begin(&mb)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            for (k, v) in dict.iter() {
                write_value_with_structs(writer, key_type, &k, struct_map)?;
                write_value_with_structs(writer, val_type, &v, struct_map)?;
            }
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    format!("Unknown struct type: {}", struct_name))
            })?;
            let dict = value.cast::<PyDict>()?;
            serialize_struct_fields(writer, &struct_def.fields, dict, struct_map)?;
            writer.write_field_stop()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        }
    }
    Ok(())
}

fn read_value_with_structs<'py, R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    thrift_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'py>,
) -> PyResult<Py<PyAny>> {
    match thrift_type {
        ThriftType::Bool => {
            let val = reader.read_bool()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().to_owned().into_any().unbind())
        }
        ThriftType::Byte => {
            let val = reader.read_byte()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I16 => {
            let val = reader.read_i16()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I32 => {
            let val = reader.read_i32()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::I64 => {
            let val = reader.read_i64()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::Double => {
            let val = reader.read_double()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::String => {
            let val = reader.read_string()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(val.into_pyobject(py).unwrap().into_any().unbind())
        }
        ThriftType::Binary => {
            let val = reader.read_binary()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            Ok(PyBytes::new(py, &val).into_any().unbind())
        }
        ThriftType::List(elem_type) | ThriftType::Set(elem_type) => {
            let (_elem_ttype, size) = if matches!(thrift_type, ThriftType::List(_)) {
                let lb = reader.read_list_begin()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                (lb.element_type, lb.size)
            } else {
                let sb = reader.read_set_begin()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
                (sb.element_type, sb.size)
            };
            let list = PyList::empty(py);
            for _ in 0..size {
                let item = read_value_with_structs(reader, elem_type, struct_map, py)?;
                list.append(item.bind(py))?;
            }
            Ok(list.into_any().unbind())
        }
        ThriftType::Map(key_type, val_type) => {
            use pyo3::types::PyDict as PyDictType;
            let mb = reader.read_map_begin()
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            let dict = PyDictType::new(py);
            for _ in 0..mb.size {
                let k = read_value_with_structs(reader, key_type, struct_map, py)?;
                let v = read_value_with_structs(reader, val_type, struct_map, py)?;
                dict.set_item(k.bind(py), v.bind(py))?;
            }
            Ok(dict.into_any().unbind())
        }
        ThriftType::Struct(struct_name) => {
            let struct_def = struct_map.get(struct_name).ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(
                    format!("Unknown struct type: {}", struct_name))
            })?;
            let fm: HashMap<i16, usize> = struct_def.fields.iter()
                .enumerate().map(|(i, f)| (f.id, i)).collect();
            let dict = deserialize_struct_fields(reader, &struct_def.fields, &fm, struct_map, py)?;
            Ok(dict.into_any().unbind())
        }
    }
}

/// Skip over a value of the given wire type without allocating Python objects.
fn skip_value<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
    ttype: TType,
) -> std::io::Result<()> {
    match ttype {
        TType::Bool | TType::Byte => { reader.read_u8_raw()?; }
        TType::I16 => { reader.read_i16_raw()?; }
        TType::I32 => { reader.read_i32_raw()?; }
        TType::I64 | TType::Double => { reader.read_i64_raw()?; }
        TType::String => { reader.read_string()?; }
        TType::Struct => {
            loop {
                let ft = reader.read_u8_raw()?;
                let ft = TType::from_u8(ft).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
                if ft == TType::Stop { break; }
                reader.read_i16_raw()?;
                skip_value(reader, ft)?;
            }
        }
        TType::Map => {
            let key_type = TType::from_u8(reader.read_u8_raw()?).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let val_type = TType::from_u8(reader.read_u8_raw()?).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let size = reader.read_i32_raw()?;
            for _ in 0..size {
                skip_value(reader, key_type)?;
                skip_value(reader, val_type)?;
            }
        }
        TType::List | TType::Set => {
            let elem_type = TType::from_u8(reader.read_u8_raw()?).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
            let size = reader.read_i32_raw()?;
            for _ in 0..size {
                skip_value(reader, elem_type)?;
            }
        }
        _ => {}
    }
    Ok(())
}
