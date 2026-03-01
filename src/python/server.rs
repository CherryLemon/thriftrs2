// ──────────────────────────────────────────────────────────────────────────────
// server.rs  –  ThriftServer and connection-handling logic
// ──────────────────────────────────────────────────────────────────────────────
use crate::protocol::{
    BinaryProtocolReader, BinaryProtocolWriter, FieldBegin, TType,
    MESSAGE_TYPE_EXCEPTION,
};
use byteorder::BigEndian;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Cursor, Write};
use std::net::TcpStream;
use std::sync::Arc;

use super::parser::ThriftParser;
use super::serde::{
    deserialize_rust_struct, thrift_type_to_ttype, write_value_with_structs,
};
use super::types::{
    PyThriftService, RustStructValue, ThriftField, ThriftStruct, ThriftStructInstance,
    TransportType,
};

#[pyclass]
pub struct ThriftServer {
    service: PyThriftService,
    handlers: HashMap<String, Py<PyAny>>,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    workers: usize,
}

#[pymethods]
impl ThriftServer {
    #[new]
    #[pyo3(signature = (service, transport = TransportType::Framed, workers = 1))]
    pub fn new(service: PyThriftService, transport: TransportType, workers: usize) -> Self {
        Self {
            service,
            handlers: HashMap::new(),
            struct_map: Arc::new(HashMap::new()),
            transport,
            workers,
        }
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
    pub fn workers(&self) -> usize {
        self.workers
    }

    #[setter]
    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers;
    }

    pub fn set_parser(&mut self, parser: &ThriftParser) {
        self.struct_map = parser.struct_map();
    }

    pub fn register_handler(&mut self, method_name: &str, handler: Py<PyAny>) {
        self.handlers.insert(method_name.to_string(), handler);
    }

    pub fn serve(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;

        let addr = format!("{}:{}", host, port);
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self
            .handlers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let n_workers = if self.workers == 0 {
            num_cpus::get().max(2)
        } else {
            self.workers
        };

        let listener = TcpListener::bind(&addr).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e))
        })?;

        println!(
            "ThriftServer ({:?}, {} workers) listening on {}",
            transport, n_workers, addr
        );

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);

        py.detach(|| {
            run_server_pool(listener, service, handlers, struct_map, transport, n_workers);
        });

        Ok(())
    }

    pub fn serve_nonblocking(&self, py: Python<'_>, host: &str, port: u16) -> PyResult<()> {
        use std::net::TcpListener;

        let addr = format!("{}:{}", host, port);
        let service = self.service.clone();
        let handlers: HashMap<String, Py<PyAny>> = self
            .handlers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone_ref(py)))
            .collect();
        let struct_map = Arc::clone(&self.struct_map);
        let transport = self.transport;
        let n_workers = if self.workers == 0 {
            num_cpus::get().max(2)
        } else {
            self.workers
        };

        let listener = TcpListener::bind(&addr).map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyOSError, _>(format!("bind error: {}", e))
        })?;

        println!(
            "ThriftServer (non-blocking, {:?}, {} workers) listening on {}",
            transport, n_workers, addr
        );

        let service = Arc::new(service);
        let handlers = Arc::new(handlers);

        std::thread::spawn(move || {
            run_server_pool(listener, service, handlers, struct_map, transport, n_workers);
        });

        Ok(())
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Thread-pool server helpers
// ──────────────────────────────────────────────────────────────────────────────

fn run_server_pool(
    listener: std::net::TcpListener,
    service: Arc<PyThriftService>,
    handlers: Arc<HashMap<String, Py<PyAny>>>,
    struct_map: Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
    _n_workers: usize,
) {
    // Spawn one OS thread per accepted connection.
    //
    // The previous semaphore-based design caused a deadlock under high
    // concurrency: every worker thread blocks inside `Python::attach` waiting
    // to acquire the GIL, and while they are all blocked the accept loop
    // cannot hand out a new slot (because `active >= n_workers`), so no
    // progress is ever made.
    //
    // The GIL itself serialises Python execution — there is no need for an
    // additional outer gate.  Threads that are not currently running Python
    // code release the GIL automatically, so many OS threads can coexist
    // while only one holds the GIL at a time.  Blocking the accept loop on
    // top of this is both unnecessary and harmful.
    for stream in listener.incoming() {
        let stream = match stream {
            Ok(s) => s,
            Err(e) => { eprintln!("Accept error: {}", e); continue; }
        };

        let service = Arc::clone(&service);
        let handlers = Arc::clone(&handlers);
        let struct_map = Arc::clone(&struct_map);

        std::thread::spawn(move || {
            if let Err(e) =
                handle_connection(stream, &service, &handlers, &struct_map, transport)
            {
                if e.kind() != std::io::ErrorKind::UnexpectedEof
                    && e.kind() != std::io::ErrorKind::ConnectionReset
                    && e.kind() != std::io::ErrorKind::BrokenPipe
                {
                    eprintln!("Connection error: {}", e);
                }
            }
        });
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Connection handler
// ──────────────────────────────────────────────────────────────────────────────

fn handle_connection(
    stream: TcpStream,
    service: &PyThriftService,
    handlers: &HashMap<String, Py<PyAny>>,
    struct_map: &Arc<HashMap<String, ThriftStruct>>,
    transport: TransportType,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;
    use std::io::Read;

    let _ = stream.set_nodelay(true);
    let write_stream = stream.try_clone()?;
    let mut buf_reader = BufReader::with_capacity(65536, stream);
    let mut buf_writer = BufWriter::with_capacity(65536, write_stream);

    loop {
        let frame: Vec<u8> = match transport {
            TransportType::Framed => {
                let frame_len = match buf_reader.read_i32::<BigEndian>() {
                    Ok(n) if n > 0 => n as usize,
                    Ok(_) => return Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                };
                let mut buf = vec![0u8; frame_len];
                buf_reader.read_exact(&mut buf)?;
                buf
            }
            TransportType::Buffered => {
                let mut hdr = [0u8; 4];
                match buf_reader.read_exact(&mut hdr) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                    Err(e) => return Err(e),
                }
                let mut name_len_bytes = [0u8; 4];
                buf_reader.read_exact(&mut name_len_bytes)?;
                let name_len = i32::from_be_bytes(name_len_bytes) as usize;
                let mut name_buf = vec![0u8; name_len];
                buf_reader.read_exact(&mut name_buf)?;
                let mut seq_id_bytes = [0u8; 4];
                buf_reader.read_exact(&mut seq_id_bytes)?;

                let mut body: Vec<u8> = Vec::with_capacity(256);
                read_buffered_struct_body(&mut buf_reader, &mut body)?;

                let mut frame = Vec::with_capacity(4 + 4 + name_len + 4 + body.len());
                frame.extend_from_slice(&hdr);
                frame.extend_from_slice(&name_len_bytes);
                frame.extend_from_slice(&name_buf);
                frame.extend_from_slice(&seq_id_bytes);
                frame.extend_from_slice(&body);
                frame
            }
        };

        let mut cursor = Cursor::new(&frame[..]);
        let mut reader = BinaryProtocolReader::new(&mut cursor);
        let msg_begin = reader
            .read_message_begin()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        let method_def = service.methods.iter().find(|m| m.name == msg_begin.name);

        let response_payload = match method_def {
            None => build_exception_reply(
                &msg_begin.name,
                msg_begin.seq_id,
                1,
                &format!("Unknown method: {}", msg_begin.name),
            ),
            Some(method) => {
                let args_rust = deserialize_rust_struct(
                    &mut reader,
                    &method.arguments,
                    &method.arg_field_map,
                    struct_map,
                )
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

                let args_rust = RustStructValue {
                    struct_name: format!("{}_args", msg_begin.name),
                    field_names: args_rust.field_names,
                    values: args_rust.values,
                };

                let handler = match handlers.get(&msg_begin.name) {
                    Some(h) => h,
                    None => {
                        let payload = build_exception_reply(
                            &msg_begin.name,
                            msg_begin.seq_id,
                            1,
                            &format!("No handler registered for: {}", msg_begin.name),
                        );
                        send_response_buffered(&mut buf_writer, &payload, transport)?;
                        continue;
                    }
                };

                let result: Result<Vec<u8>, String> = Python::attach(|py| {
                    let schema: HashMap<String, ThriftField> = method
                        .arguments
                        .iter()
                        .map(|f| (f.name.clone(), f.clone()))
                        .collect();
                    let args_instance =
                        ThriftStructInstance::from_rust(args_rust, schema, Arc::clone(struct_map));
                    let py_args = Bound::new(py, args_instance)?;

                    let kwargs = PyDict::new(py);
                    let mut inst_borrow = py_args.borrow_mut();
                    let field_names: Vec<String> = inst_borrow.field_names.clone();
                    for name in &field_names {
                        let val = inst_borrow
                            .get_field(py, name)
                            .map(|v| v.unbind())
                            .unwrap_or_else(|| py.None());
                        kwargs.set_item(name, val)?;
                    }
                    drop(inst_borrow);

                    let result = handler.call(py, (), Some(&kwargs))?;
                    let reply_body =
                        build_reply_body(py, &method.return_type, result.bind(py), struct_map)?;
                    Ok(build_reply_frame(
                        &msg_begin.name,
                        msg_begin.seq_id,
                        &reply_body,
                    ))
                })
                .map_err(|e: PyErr| e.to_string());

                match result {
                    Ok(frame) => frame,
                    Err(err_msg) => {
                        build_exception_reply(&msg_begin.name, msg_begin.seq_id, 6, &err_msg)
                    }
                }
            }
        };

        send_response_buffered(&mut buf_writer, &response_payload, transport)?;
    }
}

fn send_response_buffered<W: Write>(
    writer: &mut W,
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

// ──────────────────────────────────────────────────────────────────────────────
// Buffered transport body reader
// ──────────────────────────────────────────────────────────────────────────────

pub(crate) fn read_buffered_struct_body<R: std::io::Read>(
    reader: &mut R,
    out: &mut Vec<u8>,
) -> std::io::Result<()> {
    use byteorder::ReadBytesExt;

    loop {
        let field_type_byte = reader.read_u8()?;
        out.push(field_type_byte);

        if field_type_byte == 0x00 {
            return Ok(());
        }

        let id_hi = reader.read_u8()?;
        let id_lo = reader.read_u8()?;
        out.push(id_hi);
        out.push(id_lo);

        read_buffered_value(reader, field_type_byte, out)?;
    }
}

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
            let mut header = [0u8; 6];
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
            let mut header = [0u8; 5];
            reader.read_exact(&mut header)?;
            out.extend_from_slice(&header);
            let elem_type = header[0];
            let size = i32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            for _ in 0..size {
                read_buffered_value(reader, elem_type, out)?;
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unknown field type byte 0x{:02x} in buffered stream",
                    field_type_byte
                ),
            ));
        }
    }
    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────────
// Reply builders
// ──────────────────────────────────────────────────────────────────────────────

use crate::parser::ast::ThriftType;

fn build_reply_body(
    py: Python<'_>,
    return_type: &ThriftType,
    value: &Bound<'_, PyAny>,
    struct_map: &HashMap<String, ThriftStruct>,
) -> PyResult<Vec<u8>> {
    let mut buf = Vec::with_capacity(64);
    let mut writer = BinaryProtocolWriter::new(&mut buf);

    match return_type {
        ThriftType::Struct(name) if name == "void" => {}
        _ => {
            let field_begin = FieldBegin {
                name: None,
                field_type: thrift_type_to_ttype(return_type),
                id: 0,
            };
            writer
                .write_field_begin(&field_begin)
                .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
            write_value_with_structs(&mut writer, return_type, value, struct_map)?;
        }
    }

    writer
        .write_field_stop()
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyIOError, _>(e.to_string()))?;
    drop(writer);
    let _ = py;
    Ok(buf)
}

fn build_reply_frame(method_name: &str, seq_id: i32, body: &[u8]) -> Vec<u8> {
    use crate::protocol::MESSAGE_TYPE_REPLY;
    let mut buf = Vec::with_capacity(32 + body.len());
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(method_name, MESSAGE_TYPE_REPLY, seq_id)
            .unwrap();
    }
    buf.extend_from_slice(body);
    buf
}

fn build_exception_reply(method_name: &str, seq_id: i32, ex_type: i32, message: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64);
    {
        let mut writer = BinaryProtocolWriter::new(&mut buf);
        writer
            .write_message_begin(method_name, MESSAGE_TYPE_EXCEPTION, seq_id)
            .unwrap();
        let msg_field = FieldBegin {
            name: None,
            field_type: TType::String,
            id: 1,
        };
        writer.write_field_begin(&msg_field).unwrap();
        writer.write_string(message).unwrap();
        let type_field = FieldBegin {
            name: None,
            field_type: TType::I32,
            id: 2,
        };
        writer.write_field_begin(&type_field).unwrap();
        writer.write_i32(ex_type).unwrap();
        writer.write_field_stop().unwrap();
    }
    buf
}

