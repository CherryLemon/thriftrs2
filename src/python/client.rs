// ──────────────────────────────────────────────────────────────────────────────
// client.rs  –  ThriftClient, ThriftApplicationException and I/O helpers
//               (tokio async TCP)
// ──────────────────────────────────────────────────────────────────────────────
use crate::parser::ast::ThriftType;
use crate::protocol::{
    BinaryProtocolReader, BinaryProtocolWriter, CompactProtocolReader, CompactProtocolWriter,
    FieldBegin, JSONProtocolReader, JSONProtocolWriter, MessageBegin, TInputProtocol,
    TOutputProtocol, TType, MESSAGE_TYPE_CALL, MESSAGE_TYPE_EXCEPTION, MESSAGE_TYPE_ONEWAY,
};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::runtime::Runtime;

use super::parser::{ProtocolType, ThriftParser};
use super::serde::{
    read_value_with_structs, skip_value, thrift_type_to_ttype, write_value_with_structs,
};
use super::types::{PyThriftService, ThriftField, ThriftStruct, TransportType};

// ──────────────────────────────────────────────────────────────────────────────
// ThriftApplicationException
// ──────────────────────────────────────────────────────────────────────────────

/// A Thrift application-level exception returned by the remote server.
#[pyclass(extends = pyo3::exceptions::PyException)]
#[derive(Debug)]
pub struct ThriftApplicationException {
    #[pyo3(get)]
    pub message: String,
    #[pyo3(get)]
    pub type_: i32,
}

#[pymethods]
impl ThriftApplicationException {
    #[new]
    pub fn new(message: String, type_: i32) -> Self {
        ThriftApplicationException {
            message: message.clone(),
            type_,
        }
    }

    pub fn __repr__(&self) -> String {
        format!(
            "ThriftApplicationException(type={}, message={:?})",
            self.type_, self.message
        )
    }

    pub fn __str__(&self) -> String {
        self.message.clone()
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// ThriftClient
// ──────────────────────────────────────────────────────────────────────────────

/// Async I/O state for an open connection (held inside the Mutex).
struct Connection {
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
}

/// An async Thrift Binary Protocol client backed by Tokio.
#[pyclass]
pub struct ThriftClient {
    service: PyThriftService,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    protocol: ProtocolType,
    host: String,
    port: u16,
    conn: Mutex<Option<Connection>>,
    seq_id: AtomicI32,
    /// Pre-computed method name -> index in service.methods for O(1) lookup.
    method_index: HashMap<String, usize>,
    /// Each ThriftClient owns its own single-threaded Tokio runtime so that
    /// `open` / `call` / `close` can be called from any Python thread without
    /// requiring a running async context.
    rt: Runtime,
}

#[pymethods]
impl ThriftClient {
    #[new]
    #[pyo3(signature = (service, host, port, transport = TransportType::Framed, protocol = ProtocolType::Binary))]
    pub fn new(
        service: PyThriftService,
        host: String,
        port: u16,
        transport: TransportType,
        protocol: ProtocolType,
    ) -> PyResult<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("tokio runtime: {}", e))
            })?;
        let method_index: HashMap<String, usize> = service
            .methods
            .iter()
            .enumerate()
            .map(|(i, m)| (m.name.clone(), i))
            .collect();
        Ok(Self {
            service,
            struct_map: Arc::new(HashMap::new()),
            transport,
            protocol,
            host,
            port,
            conn: Mutex::new(None),
            seq_id: AtomicI32::new(0),
            method_index,
            rt,
        })
    }

    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    pub fn open(&self, py: Python<'_>) -> PyResult<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream: TcpStream = py
            .detach(|| self.rt.block_on(TcpStream::connect(&addr)))
            .map_err(|e| {
                PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("connect to {}: {}", addr, e))
            })?;
        let _ = stream.set_nodelay(true);
        let (read_half, write_half) = stream.into_split();
        let mut guard = self.conn.lock().unwrap();
        *guard = Some(Connection {
            reader: BufReader::with_capacity(65536, read_half),
            writer: BufWriter::with_capacity(65536, write_half),
        });
        Ok(())
    }

    pub fn close(&self) {
        let mut guard = self.conn.lock().unwrap();
        *guard = None;
    }

    pub fn is_open(&self) -> bool {
        self.conn.lock().unwrap().is_some()
    }

    pub fn __enter__<'a>(slf: PyRef<'a, Self>, py: Python<'a>) -> PyResult<PyRef<'a, Self>> {
        slf.open(py)?;
        Ok(slf)
    }

    pub fn __exit__(
        &self,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) {
        self.close();
    }

    #[getter]
    pub fn transport(&self) -> TransportType {
        self.transport
    }

    #[setter]
    pub fn set_transport(&mut self, transport: TransportType) {
        self.transport = transport;
    }

    #[getter]
    pub fn protocol(&self) -> ProtocolType {
        self.protocol
    }

    #[setter]
    pub fn set_protocol(&mut self, protocol: ProtocolType) {
        self.protocol = protocol;
    }

    #[pyo3(signature = (method_name, **kwargs))]
    pub fn call(
        &self,
        py: Python<'_>,
        method_name: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        // O(1) method lookup via pre-computed index map.
        let method_idx = self.method_index.get(method_name).ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                "Unknown method: {}",
                method_name
            ))
        })?;
        let method = &self.service.methods[*method_idx];

        let seq_id = self.seq_id.fetch_add(1, Ordering::Relaxed);
        let message_type = if method.oneway {
            MESSAGE_TYPE_ONEWAY
        } else {
            MESSAGE_TYPE_CALL
        };

        // ── Phase 1: serialise the call frame in a single writer pass ─────────
        let call_frame: Vec<u8> = {
            let empty_dict;
            let kw: &Bound<'_, PyDict> = if let Some(k) = kwargs {
                k
            } else {
                empty_dict = PyDict::new(py);
                &empty_dict
            };

            let mut buf = Vec::with_capacity(256);
            match self.protocol {
                ProtocolType::Binary => {
                    let mut writer = BinaryProtocolWriter::new(&mut buf);
                    write_call_frame(
                        &mut writer,
                        method_name,
                        message_type,
                        seq_id,
                        &method.arguments,
                        kw,
                        &self.struct_map,
                    )?;
                }
                ProtocolType::Compact => {
                    let mut writer = CompactProtocolWriter::new(&mut buf);
                    write_call_frame(
                        &mut writer,
                        method_name,
                        message_type,
                        seq_id,
                        &method.arguments,
                        kw,
                        &self.struct_map,
                    )?;
                }
                ProtocolType::JSON => {
                    let mut writer = JSONProtocolWriter::new(&mut buf);
                    write_call_frame(
                        &mut writer,
                        method_name,
                        message_type,
                        seq_id,
                        &method.arguments,
                        kw,
                        &self.struct_map,
                    )?;
                }
            }
            buf
        };

        let return_type = method.return_type.clone();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let protocol = self.protocol;
        let is_oneway = method.oneway;

        // ── Phase 2: async send (and maybe recv) without the GIL ─────────────
        if is_oneway {
            py.detach(|| -> Result<(), String> {
                let mut guard = self.conn.lock().unwrap();
                let conn = guard.as_mut().ok_or_else(|| {
                    "ThriftClient is not open; call client.open() first".to_string()
                })?;
                self.rt
                    .block_on(async {
                        conn_send_frame(&mut conn.writer, &call_frame, transport).await
                    })
                    .map_err(|e| format!("I/O error: {}", e))
            })
            .map_err(PyErr::new::<pyo3::exceptions::PyOSError, _>)?;
            return Ok(py.None());
        }

        let reply_payload: Vec<u8> = py
            .detach(|| -> Result<Vec<u8>, String> {
                let mut guard = self.conn.lock().unwrap();
                let conn = guard.as_mut().ok_or_else(|| {
                    "ThriftClient is not open; call client.open() first".to_string()
                })?;
                self.rt
                    .block_on(async {
                        conn_send_frame(&mut conn.writer, &call_frame, transport).await?;
                        conn_recv_frame(&mut conn.reader, transport, protocol).await
                    })
                    .map_err(|e| format!("I/O error: {}", e))
            })
            .map_err(PyErr::new::<pyo3::exceptions::PyOSError, _>)?;

        // ── Phase 3: decode reply directly into Python objects ────────────────
        let mut cursor = Cursor::new(&reply_payload[..]);
        match protocol {
            ProtocolType::Binary => {
                let mut reader = BinaryProtocolReader::new(&mut cursor);
                decode_reply(&mut reader, &return_type, &struct_map, py)
            }
            ProtocolType::Compact => {
                let mut reader = CompactProtocolReader::new(&mut cursor);
                decode_reply(&mut reader, &return_type, &struct_map, py)
            }
            ProtocolType::JSON => {
                let mut reader = JSONProtocolReader::new(&mut cursor);
                decode_reply(&mut reader, &return_type, &struct_map, py)
            }
        }
    }
}

fn write_call_frame<P: TOutputProtocol>(
    writer: &mut P,
    method_name: &str,
    message_type: u8,
    seq_id: i32,
    arguments: &[ThriftField],
    kwargs: &Bound<'_, PyDict>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<()> {
    writer
        .write_message_begin(&MessageBegin {
            name: method_name.to_string(),
            message_type,
            seq_id,
        })
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    for field in arguments {
        let value = match kwargs.get_item(&field.name)? {
            Some(v) => v,
            None => continue,
        };
        let fb = FieldBegin {
            name: None,
            field_type: thrift_type_to_ttype(&field.field_type),
            id: field.id,
        };
        writer
            .write_field_begin(&fb)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        write_value_with_structs(writer, &field.field_type, &value, struct_map)?;
        writer
            .write_field_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    }
    writer
        .write_field_stop()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    writer
        .write_message_end()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(())
}

fn decode_reply<P: TInputProtocol>(
    reader: &mut P,
    return_type: &ThriftType,
    struct_map: &HashMap<String, ThriftStruct>,
    py: Python<'_>,
) -> PyResult<Py<PyAny>> {
    let msg_begin = reader
        .read_message_begin()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

    if msg_begin.message_type == MESSAGE_TYPE_EXCEPTION {
        let (ex_msg, ex_type) = read_application_exception(reader)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        reader
            .read_message_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        return Err(PyErr::new::<ThriftApplicationException, _>((
            ex_msg, ex_type,
        )));
    }

    let field_begin = reader
        .read_field_begin()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

    if field_begin.field_type == TType::Stop {
        reader
            .read_message_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        return Ok(py.None());
    }

    if field_begin.id != 0 {
        let ex_py = read_value_with_structs(reader, return_type, struct_map, py)
            .unwrap_or_else(|_| py.None());
        reader
            .read_field_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        reader
            .read_message_end()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
            "Service exception (field {}): {:?}",
            field_begin.id,
            ex_py
                .bind(py)
                .repr()
                .map(|s| s.to_string())
                .unwrap_or_default()
        )));
    }

    let value = read_value_with_structs(reader, return_type, struct_map, py)?;
    reader
        .read_field_end()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    reader
        .read_message_end()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    Ok(value)
}

// ──────────────────────────────────────────────────────────────────────────────
// Async client I/O helpers  (GIL-free, no Python types)
// ──────────────────────────────────────────────────────────────────────────────

/// Write a framed or buffered message and flush.
async fn conn_send_frame(
    writer: &mut BufWriter<OwnedWriteHalf>,
    payload: &[u8],
    transport: TransportType,
) -> std::io::Result<()> {
    match transport {
        TransportType::Framed => {
            writer.write_i32(payload.len() as i32).await?;
            writer.write_all(payload).await?;
        }
        TransportType::Buffered => {
            writer.write_all(payload).await?;
        }
    }
    writer.flush().await
}

/// Read a complete reply frame.
async fn conn_recv_frame(
    reader: &mut BufReader<OwnedReadHalf>,
    transport: TransportType,
    protocol: ProtocolType,
) -> std::io::Result<Vec<u8>> {
    match transport {
        TransportType::Framed => {
            let frame_len = match reader.read_i32().await {
                Ok(n) if n >= 0 => n as usize,
                Ok(_) => return Ok(vec![]),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(vec![]),
                Err(e) => return Err(e),
            };
            let mut buf = vec![0u8; frame_len];
            reader.read_exact(&mut buf).await?;
            Ok(buf)
        }
        TransportType::Buffered if protocol == ProtocolType::JSON => {
            read_json_value_async(reader).await
        }
        TransportType::Buffered if protocol == ProtocolType::Binary => {
            // Read the Binary protocol message header manually, then read
            // the struct body using the sync helper (via Cursor over a buffer).
            let mut hdr = [0u8; 4];
            reader.read_exact(&mut hdr).await?;
            let mut name_len_bytes = [0u8; 4];
            reader.read_exact(&mut name_len_bytes).await?;
            let name_len = i32::from_be_bytes(name_len_bytes) as usize;
            let mut name_buf = vec![0u8; name_len];
            reader.read_exact(&mut name_buf).await?;
            let mut seq_id_bytes = [0u8; 4];
            reader.read_exact(&mut seq_id_bytes).await?;

            // Read field-by-field using the async helper.
            let mut body: Vec<u8> = Vec::with_capacity(256);
            read_buffered_struct_body_async(reader, &mut body).await?;

            let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
            frame.extend_from_slice(&hdr);
            frame.extend_from_slice(&name_len_bytes);
            frame.extend_from_slice(&name_buf);
            frame.extend_from_slice(&seq_id_bytes);
            frame.extend_from_slice(&body);
            Ok(frame)
        }
        TransportType::Buffered => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "buffered RPC transport currently supports Binary and JSON protocols",
        )),
    }
}

async fn read_json_value_async<R>(reader: &mut R) -> std::io::Result<Vec<u8>>
where
    R: AsyncReadExt + Unpin,
{
    let mut out = Vec::with_capacity(256);
    let mut started = false;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;

    loop {
        let byte = match reader.read_u8().await {
            Ok(byte) => byte,
            Err(err) if err.kind() == std::io::ErrorKind::UnexpectedEof && !started => {
                return Ok(Vec::new());
            }
            Err(err) => return Err(err),
        };

        if !started {
            if byte.is_ascii_whitespace() {
                continue;
            }
            match byte {
                b'[' | b'{' => {
                    started = true;
                    depth = 1;
                    out.push(byte);
                    continue;
                }
                _ => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "expected JSON object or array at start of buffered message",
                    ));
                }
            }
        }

        out.push(byte);

        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(out);
                }
            }
            _ => {}
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Async buffered-transport body reader (duplicated from server for client use)
// ──────────────────────────────────────────────────────────────────────────────

async fn read_buffered_struct_body_async<R>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> std::io::Result<()>
where
    R: AsyncReadExt + Unpin,
{
    loop {
        let field_type_byte = reader.read_u8().await?;
        out.push(field_type_byte);
        if field_type_byte == 0x00 {
            return Ok(());
        }
        let id_hi = reader.read_u8().await?;
        let id_lo = reader.read_u8().await?;
        out.push(id_hi);
        out.push(id_lo);
        read_buffered_value_async(reader, field_type_byte, out).await?;
    }
}

async fn read_buffered_value_async<R>(
    reader: &mut R,
    field_type_byte: u8,
    out: &mut Vec<u8>,
) -> std::io::Result<()>
where
    R: AsyncReadExt + Unpin,
{
    match field_type_byte {
        0x02 | 0x03 => {
            let b = reader.read_u8().await?;
            out.push(b);
        }
        0x06 => {
            let mut buf = [0u8; 2];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        0x08 => {
            let mut buf = [0u8; 4];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        0x0a | 0x04 => {
            let mut buf = [0u8; 8];
            reader.read_exact(&mut buf).await?;
            out.extend_from_slice(&buf);
        }
        0x0b => {
            let mut len_bytes = [0u8; 4];
            reader.read_exact(&mut len_bytes).await?;
            out.extend_from_slice(&len_bytes);
            let len = u32::from_be_bytes(len_bytes) as usize;
            let start = out.len();
            out.resize(start + len, 0);
            reader.read_exact(&mut out[start..]).await?;
        }
        0x0c => {
            Box::pin(read_buffered_struct_body_async(reader, out)).await?;
        }
        0x0d => {
            let mut header = [0u8; 6];
            reader.read_exact(&mut header).await?;
            out.extend_from_slice(&header);
            let key_type = header[0];
            let val_type = header[1];
            let size = i32::from_be_bytes([header[2], header[3], header[4], header[5]]);
            for _ in 0..size {
                Box::pin(read_buffered_value_async(reader, key_type, out)).await?;
                Box::pin(read_buffered_value_async(reader, val_type, out)).await?;
            }
        }
        0x0f | 0x0e => {
            let mut header = [0u8; 5];
            reader.read_exact(&mut header).await?;
            out.extend_from_slice(&header);
            let elem_type = header[0];
            let size = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            for _ in 0..size {
                Box::pin(read_buffered_value_async(reader, elem_type, out)).await?;
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unknown field type byte 0x{:02x}", field_type_byte),
            ));
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Application-exception decoder (sync, cursor-based — no change needed)
// ──────────────────────────────────────────────────────────────────────────────

fn read_application_exception<P: TInputProtocol>(reader: &mut P) -> std::io::Result<(String, i32)> {
    let mut msg = String::new();
    let mut type_code: i32 = 0;
    loop {
        let field = reader.read_field_begin()?;
        if field.field_type == TType::Stop {
            break;
        }
        match (field.field_type, field.id) {
            (TType::String, 1) => {
                msg = reader.read_string()?;
            }
            (TType::I32, 2) => {
                type_code = reader.read_i32()?;
            }
            _ => {
                skip_value(reader, field.field_type).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
            }
        }
        reader.read_field_end()?;
    }
    Ok((msg, type_code))
}
