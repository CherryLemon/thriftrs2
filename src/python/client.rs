// ──────────────────────────────────────────────────────────────────────────────
// client.rs  –  ThriftClient, ThriftApplicationException and I/O helpers
// ──────────────────────────────────────────────────────────────────────────────
use crate::protocol::{
    BinaryProtocolReader, BinaryProtocolWriter, FieldBegin, TType, MESSAGE_TYPE_CALL,
    MESSAGE_TYPE_EXCEPTION,
};
use byteorder::BigEndian;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Cursor, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use super::parser::ThriftParser;
use super::serde::{read_rust_value, skip_value, thrift_type_to_ttype, thrift_value_to_py, write_value_with_structs};
use super::server::read_buffered_struct_body;
use super::types::{PyThriftService, ThriftStruct, TransportType};

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

/// Persistent, buffered I/O state for an open connection.
struct Connection {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

/// A synchronous Thrift Binary Protocol client.
#[pyclass]
pub struct ThriftClient {
    service: PyThriftService,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    host: String,
    port: u16,
    conn: Mutex<Option<Connection>>,
    seq_id: AtomicI32,
}

#[pymethods]
impl ThriftClient {
    #[new]
    #[pyo3(signature = (service, host, port, transport = TransportType::Framed))]
    pub fn new(
        service: PyThriftService,
        host: String,
        port: u16,
        transport: TransportType,
    ) -> Self {
        Self {
            service,
            struct_map: Arc::new(HashMap::new()),
            transport,
            host,
            port,
            conn: Mutex::new(None),
            seq_id: AtomicI32::new(0),
        }
    }

    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    pub fn open(&self, py: Python<'_>) -> PyResult<()> {
        let addr = format!("{}:{}", self.host, self.port);
        let stream = py.detach(|| TcpStream::connect(&addr)).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("connect to {}: {}", addr, e))
        })?;
        let _ = stream.set_nodelay(true);
        let write_stream = stream.try_clone().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("clone socket: {}", e))
        })?;
        let mut guard = self.conn.lock().unwrap();
        *guard = Some(Connection {
            reader: BufReader::with_capacity(65536, stream),
            writer: BufWriter::with_capacity(65536, write_stream),
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

    #[pyo3(signature = (method_name, **kwargs))]
    pub fn call(
        &self,
        py: Python<'_>,
        method_name: &str,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let method = self
            .service
            .methods
            .iter()
            .find(|m| m.name == method_name)
            .ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                    "Unknown method: {}",
                    method_name
                ))
            })?
            .clone();

        let seq_id = self.seq_id.fetch_add(1, Ordering::Relaxed);

        let call_frame: Vec<u8> = {
            let empty_dict;
            let kw: &Bound<'_, PyDict> = if let Some(k) = kwargs {
                k
            } else {
                empty_dict = PyDict::new(py);
                &empty_dict
            };

            let mut buf = Vec::with_capacity(256);
            {
                let mut writer = BinaryProtocolWriter::new(&mut buf);
                writer
                    .write_message_begin(method_name, MESSAGE_TYPE_CALL, seq_id)
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }
            {
                let mut writer = BinaryProtocolWriter::new(&mut buf);
                for field in &method.arguments {
                    let value = match kw.get_item(&field.name)? {
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
                    write_value_with_structs(
                        &mut writer,
                        &field.field_type,
                        &value,
                        &self.struct_map,
                    )?;
                }
                writer
                    .write_field_stop()
                    .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            }
            buf
        };

        let return_type = method.return_type.clone();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;

        // ── Phase 2: send + recv without holding the GIL ──────────────────────
        // The conn Mutex is held only for the network I/O, which itself is done
        // without the GIL so other Python threads can run concurrently.
        let reply_payload: Vec<u8> = py
            .detach(|| -> Result<Vec<u8>, String> {
                let mut guard = self.conn.lock().unwrap();
                let conn = guard.as_mut().ok_or_else(|| {
                    "ThriftClient is not open; call client.open() first".to_string()
                })?;
                conn_send_frame(&mut conn.writer, &call_frame, transport)
                    .map_err(|e| format!("send error: {}", e))?;
                conn_recv_frame(&mut conn.reader, transport)
                    .map_err(|e| format!("recv error: {}", e))
            })
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyOSError, _>(e))?;

        // ── Phase 3: decode reply (needs GIL for Python object creation) ───────
        let mut cursor = Cursor::new(&reply_payload[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);

        let msg_begin = reader
            .read_message_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        if msg_begin.message_type == MESSAGE_TYPE_EXCEPTION {
            let (ex_msg, ex_type) = read_application_exception(&mut reader)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            return Err(PyErr::new::<ThriftApplicationException, _>((
                ex_msg, ex_type,
            )));
        }

        let field_begin = reader
            .read_field_begin()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;

        if field_begin.field_type == TType::Stop {
            return Ok(py.None());
        }

        if field_begin.id != 0 {
            let ex_val = read_rust_value(&mut reader, &return_type, &struct_map)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()));
            let ex_py = match ex_val {
                Ok(tv) => thrift_value_to_py(&tv, py)?,
                Err(_) => py.None(),
            };
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

        let rust_val = read_rust_value(&mut reader, &return_type, &struct_map)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
        thrift_value_to_py(&rust_val, py)
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Client I/O helpers  (GIL-free, no Python types)
// ──────────────────────────────────────────────────────────────────────────────

/// Write a framed or buffered message through the persistent BufWriter.
fn conn_send_frame(
    writer: &mut BufWriter<TcpStream>,
    payload: &[u8],
    transport: TransportType,
) -> std::io::Result<()> {
    use byteorder::WriteBytesExt;
    match transport {
        TransportType::Framed => {
            writer.write_i32::<BigEndian>(payload.len() as i32)?;
            writer.write_all(payload)?;
        }
        TransportType::Buffered => {
            writer.write_all(payload)?;
        }
    }
    writer.flush()
}

/// Read a complete reply frame from the persistent BufReader.
fn conn_recv_frame(
    reader: &mut BufReader<TcpStream>,
    transport: TransportType,
) -> std::io::Result<Vec<u8>> {
    use byteorder::ReadBytesExt;
    use std::io::Read;

    match transport {
        TransportType::Framed => {
            let frame_len = match reader.read_i32::<BigEndian>() {
                Ok(n) if n >= 0 => n as usize,
                Ok(_) => return Ok(vec![]),
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(vec![]),
                Err(e) => return Err(e),
            };
            let mut buf = vec![0u8; frame_len];
            reader.read_exact(&mut buf)?;
            Ok(buf)
        }
        TransportType::Buffered => {
            let mut hdr = [0u8; 4];
            reader.read_exact(&mut hdr)?;
            let mut name_len_bytes = [0u8; 4];
            reader.read_exact(&mut name_len_bytes)?;
            let name_len = i32::from_be_bytes(name_len_bytes) as usize;
            let mut name_buf = vec![0u8; name_len];
            reader.read_exact(&mut name_buf)?;
            let mut seq_id_bytes = [0u8; 4];
            reader.read_exact(&mut seq_id_bytes)?;
            let mut body: Vec<u8> = Vec::with_capacity(256);
            read_buffered_struct_body(reader, &mut body)?;
            let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
            frame.extend_from_slice(&hdr);
            frame.extend_from_slice(&name_len_bytes);
            frame.extend_from_slice(&name_buf);
            frame.extend_from_slice(&seq_id_bytes);
            frame.extend_from_slice(&body);
            Ok(frame)
        }
    }
}

fn read_application_exception<R: std::io::Read>(
    reader: &mut BinaryProtocolReader<R>,
) -> std::io::Result<(String, i32)> {
    let mut msg = String::new();
    let mut type_code: i32 = 0;
    loop {
        let ft = reader.read_u8_raw()?;
        let ttype = TType::from_u8(ft)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "bad type"))?;
        if ttype == TType::Stop {
            break;
        }
        let id = reader.read_i16_raw()?;
        match (ttype, id) {
            (TType::String, 1) => {
                msg = reader.read_string()?;
            }
            (TType::I32, 2) => {
                type_code = reader.read_i32_raw()?;
            }
            _ => {
                skip_value(reader, ttype).map_err(|e| {
                    std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string())
                })?;
            }
        }
    }
    Ok((msg, type_code))
}

